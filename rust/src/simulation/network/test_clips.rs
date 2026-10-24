// SPDX-License-Identifier: GPL-2.0-only

#[cfg(test)]
mod tests {
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::roadbed::{EdgeEnd, NodeArms};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use godot::prelude::Vector3;

    // Each arm's mouth station in the node's junction layout (`ROAD-68-C`), by edge and end.
    fn layout_mouths(g: &RegionGraph, node: u32) -> Vec<(usize, bool, f32)> {
        let arms = NodeArms::collect(g, node).expect("node has a layout");
        let layout = arms.layout().expect("node lays out");
        layout
            .arms
            .iter()
            .map(|arm| {
                let at_start = arm.key.end == EdgeEnd::Start;
                (arm.key.edge, at_start, arm.mouth_station_m as f32)
            })
            .collect()
    }

    fn clip(g: &RegionGraph, edge: usize, at_start: bool) -> f32 {
        if at_start {
            g.edges[edge].start_clip
        } else {
            g.edges[edge].end_clip
        }
    }

    fn road_edge(start_node: u32, end_node: u32, start: Vector3, end: Vector3, width: f32) -> Edge {
        Edge {
            start_node,
            end_node,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![start, end],
            physical_geometry: vec![],
            class: EdgeClass::Standard,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        }
    }

    #[test]
    fn test_t_junction_clipping() {
        let mut g = RegionGraph::new();
        // T intersection at (0,0) resulting from a split
        let n_center = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);

        let n_left = g.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
        let n_right = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let n_bot = g.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);

        // Edge 0: Left to Center (-X to 0) (Horizontal Main Road Part 1)
        g.add_edge(Edge {
            start_node: n_left,
            end_node: n_center,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 2.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![
                Vector3::new(-10.0, 0.0, 0.0),
                Vector3::new(-5.0, 0.0, 0.0),
                Vector3::new(0.0, 0.0, 0.0),
            ],
            physical_geometry: vec![],
            class: EdgeClass::Standard,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        });
        // Edge 1: Center to Right (0 to +X) (Horizontal Main Road Part 2)
        g.add_edge(Edge {
            start_node: n_center,
            end_node: n_right,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 2.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(5.0, 0.0, 0.0),
                Vector3::new(10.0, 0.0, 0.0),
            ],
            physical_geometry: vec![],
            class: EdgeClass::Standard,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        });
        // Edge 2: Bot to Center (+Z to 0) (Vertical Road connecting)
        g.add_edge(Edge {
            start_node: n_bot,
            end_node: n_center,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 2.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![
                Vector3::new(0.0, 0.0, 10.0),
                Vector3::new(0.0, 0.0, 5.0),
                Vector3::new(0.0, 0.0, 0.0),
            ],
            physical_geometry: vec![],
            class: EdgeClass::Standard,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        });

        g.rebuild_intersection_clips();

        assert!(g.edges[0].end_clip > 0.0);
        assert!(g.edges[1].start_clip > 0.0);
        assert!(g.edges[2].end_clip > 0.0);
    }

    #[test]
    fn test_acute_t_junction_uses_angle_aware_clips() {
        let mut g = RegionGraph::new();
        let center = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let left = g.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
        let right = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let branch = g.add_node(
            Vector3::new(
                30.0_f32.to_radians().cos() * 100.0,
                0.0,
                30.0_f32.to_radians().sin() * 100.0,
            ),
            NodeType::Junction,
        );

        g.add_edge(road_edge(
            left,
            center,
            Vector3::new(-100.0, 0.0, 0.0),
            Vector3::ZERO,
            7.0,
        ));
        g.add_edge(road_edge(
            center,
            right,
            Vector3::ZERO,
            Vector3::new(100.0, 0.0, 0.0),
            7.0,
        ));
        g.add_edge(road_edge(
            branch,
            center,
            Vector3::new(
                30.0_f32.to_radians().cos() * 100.0,
                0.0,
                30.0_f32.to_radians().sin() * 100.0,
            ),
            Vector3::ZERO,
            7.0,
        ));

        g.rebuild_intersection_clips();

        // Clips are the layout's mouth stations; the 30 degree corner reaches far down the branch.
        for (edge, at_start, mouth_m) in layout_mouths(&g, center) {
            assert_eq!(clip(&g, edge, at_start), mouth_m);
        }
        assert!(
            g.edges[2].end_clip > g.edges[0].end_clip.min(g.edges[1].start_clip),
            "acute branch clip did not expand beyond the obtuse side: {}",
            g.edges[2].end_clip
        );
    }

    #[test]
    fn test_orthogonal_junction_clip_uses_small_roadbed_margin() {
        let mut g = RegionGraph::new();
        let center = g.add_node(Vector3::ZERO, NodeType::Junction);
        let west = g.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
        let east = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let south = g.add_node(Vector3::new(0.0, 0.0, -100.0), NodeType::Junction);
        let north = g.add_node(Vector3::new(0.0, 0.0, 100.0), NodeType::Junction);

        g.add_edge(road_edge(
            west,
            center,
            Vector3::new(-100.0, 0.0, 0.0),
            Vector3::ZERO,
            7.0,
        ));
        g.add_edge(road_edge(
            center,
            east,
            Vector3::ZERO,
            Vector3::new(100.0, 0.0, 0.0),
            7.0,
        ));
        g.add_edge(road_edge(
            south,
            center,
            Vector3::new(0.0, 0.0, -100.0),
            Vector3::ZERO,
            7.0,
        ));
        g.add_edge(road_edge(
            center,
            north,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 100.0),
            7.0,
        ));

        g.rebuild_intersection_clips();

        // Right-angle curb returns of 3.5 m beyond the 3.5 m carriageway edge, plus the 3 m
        // crosswalk of a crossable arm (D1, D8).
        for (edge, at_start, mouth_m) in layout_mouths(&g, center) {
            let clip_m = clip(&g, edge, at_start);
            assert_eq!(clip_m, mouth_m);
            assert!(
                (clip_m - 10.0).abs() <= 1.0e-4,
                "orthogonal junction edge {edge} clips at its layout mouth, got {clip_m:.3}"
            );
        }
    }

    #[test]
    fn test_acute_bend_uses_same_clip_rule_as_junction() {
        let mut g = RegionGraph::new();
        let center = g.add_node(Vector3::ZERO, NodeType::Junction);
        let east = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let branch = g.add_node(
            Vector3::new(
                30.0_f32.to_radians().cos() * 100.0,
                0.0,
                30.0_f32.to_radians().sin() * 100.0,
            ),
            NodeType::Junction,
        );

        g.add_edge(road_edge(
            center,
            east,
            Vector3::ZERO,
            Vector3::new(100.0, 0.0, 0.0),
            7.0,
        ));
        g.add_edge(road_edge(
            center,
            branch,
            Vector3::ZERO,
            Vector3::new(
                30.0_f32.to_radians().cos() * 100.0,
                0.0,
                30.0_f32.to_radians().sin() * 100.0,
            ),
            7.0,
        ));

        g.rebuild_intersection_clips();

        assert!(
            g.edges[0].start_clip > 12.0,
            "acute bend clip stayed fixed-width: {}",
            g.edges[0].start_clip
        );
        assert!(
            g.edges[1].start_clip > 12.0,
            "acute bend clip stayed fixed-width: {}",
            g.edges[1].start_clip
        );
    }

    #[test]
    fn test_one_degree_conflict_has_no_layout_and_no_clip() {
        let mut g = RegionGraph::new();
        let center = g.add_node(Vector3::ZERO, NodeType::Junction);
        let east = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let near_parallel = g.add_node(
            Vector3::new(
                1.0_f32.to_radians().cos() * 100.0,
                0.0,
                1.0_f32.to_radians().sin() * 100.0,
            ),
            NodeType::Junction,
        );

        g.add_edge(road_edge(
            center,
            east,
            Vector3::ZERO,
            Vector3::new(100.0, 0.0, 0.0),
            7.0,
        ));
        g.add_edge(road_edge(
            center,
            near_parallel,
            Vector3::ZERO,
            Vector3::new(
                1.0_f32.to_radians().cos() * 100.0,
                0.0,
                1.0_f32.to_radians().sin() * 100.0,
            ),
            7.0,
        ));

        g.rebuild_intersection_clips();

        // Arms a degree apart have no layout (a placement rejection), so the node keeps no clip.
        let arms = NodeArms::collect(&g, center).unwrap();
        assert!(arms.layout().is_err());
        assert_eq!(g.edges[0].start_clip, 0.0);
        assert_eq!(g.edges[1].start_clip, 0.0);
    }

    #[test]
    fn test_pass_through_two_way_corridor_keeps_zero_clip() {
        let mut g = RegionGraph::new();
        let center = g.add_node(Vector3::ZERO, NodeType::Junction);
        let east = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let west = g.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);

        g.add_edge(road_edge(
            center,
            east,
            Vector3::ZERO,
            Vector3::new(100.0, 0.0, 0.0),
            7.0,
        ));
        g.add_edge(road_edge(
            center,
            west,
            Vector3::ZERO,
            Vector3::new(-100.0, 0.0, 0.0),
            7.0,
        ));

        g.rebuild_intersection_clips();

        assert_eq!(g.edges[0].start_clip, 0.0);
        assert_eq!(g.edges[1].start_clip, 0.0);
    }
}
