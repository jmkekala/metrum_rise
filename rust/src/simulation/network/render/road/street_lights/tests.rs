// SPDX-License-Identifier: GPL-2.0-only

//! Street light placement along roads and footpaths.

use super::{StreetLight, StreetLightKind};
use crate::config;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::*;

fn lights_for(roads: &[(&[Vector3], u8, u8)]) -> Vec<StreetLight> {
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut allocator = BuildingAllocator::new();
    for (road, fwd, bkw) in roads {
        network.add_road(
            &mut graph,
            road.to_vec(),
            *fwd,
            *bkw,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
    }
    let terrain = TerrainSystem::new(256, 256);
    network.generate_mesh_data(&graph, &terrain).street_lights
}

// Along +X, so the road's lateral axis is world Z.
const STREET: [Vector3; 2] = [Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)];

#[test]
fn a_residential_street_gets_evenly_spaced_lights_on_one_side_behind_the_kerb() {
    let lights = lights_for(&[(&STREET, 1, 1)]);
    assert!(
        lights.len() >= 4,
        "120 m street at 30 m spacing, got {lights:?}"
    );
    let side = lights[0].position.z.signum();
    let carriageway_half = config::LANE_WIDTH;
    let mut xs: Vec<f32> = lights.iter().map(|light| light.position.x).collect();
    xs.sort_by(f32::total_cmp);
    for light in &lights {
        assert!(matches!(
            light.kind,
            StreetLightKind::ResidentialSteel | StreetLightKind::ResidentialWood
        ));
        assert_eq!(light.position.z.signum(), side, "one side only");
        let offset = light.position.z.abs();
        assert!(
            offset > carriageway_half && offset <= carriageway_half + config::SIDEWALK_WIDTH + 0.6,
            "pole stands behind the kerb, within the sidewalk: {offset}"
        );
        // The model's +X arm turned by the yaw points back across the road.
        let arm = Vector3::new(light.yaw.cos(), 0.0, -light.yaw.sin());
        assert!(arm.z * side < -0.99, "arm points toward the road: {arm:?}");
    }
    let gaps: Vec<f32> = xs.windows(2).map(|pair| pair[1] - pair[0]).collect();
    for gap in &gaps {
        assert!(
            *gap <= 32.0 + 1e-3 && (gap - gaps[0]).abs() < 1e-3,
            "even gaps: {gaps:?}"
        );
    }
    assert!(
        xs[0] > STREET[0].x + 5.0 && xs[xs.len() - 1] < STREET[1].x - 5.0,
        "clear of the ends: {xs:?}"
    );
}

#[test]
fn a_four_lane_road_gets_main_road_lights_on_both_sides() {
    let lights = lights_for(&[(&STREET, 2, 2)]);
    assert!(
        lights
            .iter()
            .all(|light| light.kind == StreetLightKind::Main)
    );
    let left = lights.iter().filter(|light| light.position.z > 0.0).count();
    let right = lights.iter().filter(|light| light.position.z < 0.0).count();
    assert!(
        left > 0 && left == right,
        "paired on both sides: {left} / {right}"
    );
}

#[test]
fn a_footpath_gets_post_top_lights_on_its_verge() {
    let lights = lights_for(&[(&STREET, 0, 0)]);
    assert!(!lights.is_empty());
    for light in &lights {
        assert_eq!(light.kind, StreetLightKind::Path);
        assert!(
            light.position.z.abs() > 1.0,
            "beside the 2 m path, not on it"
        );
    }
}

#[test]
fn placement_is_the_same_whichever_way_the_street_was_drawn() {
    let reversed = [STREET[1], STREET[0]];
    let key = |lights: Vec<StreetLight>| {
        let mut points: Vec<(i32, i32)> = lights
            .iter()
            .map(|light| {
                (
                    (light.position.x * 100.0).round() as i32,
                    (light.position.z * 100.0).round() as i32,
                )
            })
            .collect();
        points.sort_unstable();
        points
    };
    assert_eq!(
        key(lights_for(&[(&STREET, 1, 1)])),
        key(lights_for(&[(&reversed, 1, 1)]))
    );
}

#[test]
fn a_street_drawn_as_short_edges_is_lit_as_one_street() {
    // Five 24 m edges through degree-two nodes: each alone is too short to hold a light between
    // its junction margins, so per-edge spacing left the street dark.
    let points: Vec<Vector3> = (0..=5)
        .map(|i| Vector3::new(-60.0 + 24.0 * i as f32, 0.0, 0.0))
        .collect();
    let segments: Vec<[Vector3; 2]> = points.windows(2).map(|pair| [pair[0], pair[1]]).collect();
    let roads: Vec<(&[Vector3], u8, u8)> = segments
        .iter()
        .map(|segment| (&segment[..], 1, 1))
        .collect();
    let lights = lights_for(&roads);
    let mut xs: Vec<f32> = lights.iter().map(|light| light.position.x).collect();
    xs.sort_by(f32::total_cmp);
    assert!(xs.len() >= 4, "lit along its whole length: {xs:?}");
    let gaps: Vec<f32> = xs.windows(2).map(|pair| pair[1] - pair[0]).collect();
    for gap in &gaps {
        assert!(
            *gap > 12.0 && *gap <= 30.0 + 0.5,
            "gaps within the family's bounds: {gaps:?}"
        );
    }
    assert!(
        lights
            .iter()
            .all(|light| light.position.z.signum() == lights[0].position.z.signum()),
        "one side along the whole street"
    );
}

// Minimal indexed graph/surface setup: no zoning, spawning, lanes or gameplay constructors.
fn fixture(
    points: &[Vector3],
    links: &[(usize, usize, u8, u8)],
) -> (RegionGraph, super::RoadbedSystem, TerrainSystem) {
    use crate::simulation::network::graph::{Edge, Node};
    use crate::simulation::network::types::{TransitFlags, TransitType};
    use std::collections::HashMap;

    let nodes = points
        .iter()
        .map(|&pos| Node {
            pos,
            node_type: Default::default(),
            lane_connections: HashMap::new(),
            crosswalk_overrides: HashMap::new(),
        })
        .collect();
    let edges = links
        .iter()
        .map(|&(start, end, fwd, bkw)| {
            let geometry = vec![points[start], points[end]];
            Edge {
                start_node: start as u32,
                end_node: end as u32,
                primary_type: if fwd + bkw == 0 {
                    TransitType::Foot
                } else {
                    TransitType::Road
                },
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                class: EdgeClass::Standard,
                width: if fwd + bkw == 0 {
                    2.0
                } else {
                    f32::from(fwd + bkw) * config::LANE_WIDTH
                },
                fwd_lanes: fwd,
                bkw_lanes: bkw,
                physical_length: points[start].distance_to(points[end]),
                physical_geometry: geometry.clone(),
                geometry,
                ..Edge::default()
            }
        })
        .collect();
    let mut graph = RegionGraph::new();
    graph.set_nodes_edges_for_test(nodes, edges);
    graph.rebuild_all_indices();
    let terrain = TerrainSystem::with_chunking(256, 256, 1.0, 8, 0.0);
    let mut surface = super::RoadbedSystem::new(64.0);
    surface.compile_dirty(&graph, &terrain);
    (graph, surface, terrain)
}

fn emit(
    f: &(RegionGraph, super::RoadbedSystem, TerrainSystem),
    edges: Vec<usize>,
) -> Vec<StreetLight> {
    let mut mesh = super::NetworkMeshData::new();
    let coverage = super::CompiledSurfaceCoverage {
        edge_indices: edges,
        node_ids: Vec::new(),
    };
    super::emit_street_lights(&mut mesh, &f.0, &f.1, &f.2, &coverage);
    mesh.street_lights
}

fn signature(lights: &[StreetLight]) -> Vec<[u32; 5]> {
    let mut keys: Vec<_> = lights
        .iter()
        .map(|light| {
            [
                light.position.x.to_bits(),
                light.position.y.to_bits(),
                light.position.z.to_bits(),
                light.yaw.to_bits(),
                light.kind as u32,
            ]
        })
        .collect();
    keys.sort_unstable();
    keys
}

fn assert_arm_clearance_and_partition(
    f: &(RegionGraph, super::RoadbedSystem, TerrainSystem),
    arms: &[Vec<usize>],
) {
    let lights: Vec<_> = arms.iter().map(|edges| emit(f, edges.clone())).collect();
    assert!(
        lights.iter().all(|arm| !arm.is_empty()),
        "every arm remains lit: {lights:?}"
    );
    for (i, arm) in lights.iter().enumerate() {
        for other_arm in &lights[i + 1..] {
            for pole in arm {
                for other in other_arm {
                    let distance = super::planar_distance(pole.position, other.position);
                    assert!(
                        distance >= super::MIN_POLE_GAP_M,
                        "{distance} m between arms: {pole:?} / {other:?}"
                    );
                }
            }
        }
    }
    let edges: Vec<_> = arms.iter().flatten().copied().collect();
    let whole = emit(f, edges.clone());
    let partition: Vec<_> = edges.iter().flat_map(|&edge| emit(f, vec![edge])).collect();
    assert_eq!(
        signature(&whole),
        signature(&partition),
        "individual covered edges agree"
    );
    assert_eq!(
        signature(&whole),
        signature(&emit(f, edges)),
        "repeat run agrees bit for bit"
    );
}

#[test]
fn width_change_keeps_residential_poles_clear_of_both_main_poles() {
    // Residential gets the lower edge id: family priority must still favour Main.
    let f = fixture(
        &[
            Vector3::ZERO,
            Vector3::new(-120.0, 0.0, 0.0),
            Vector3::new(120.0, 0.0, 0.0),
        ],
        &[(0, 1, 1, 1), (2, 0, 2, 2)],
    );
    assert_arm_clearance_and_partition(&f, &[vec![0], vec![1]]);
    let main_chain = super::street_chain(&f.0, 1, super::Family::Main);
    let main_runs = super::chain_runs(&f.0, &f.1, &f.2, &main_chain).unwrap();
    let mouth = if main_chain[0].1 {
        main_runs[0].sections[0].center_xz.x
    } else {
        main_runs[0].sections.last().unwrap().center_xz.x
    };
    let nearest = emit(&f, vec![1])
        .iter()
        .map(|pole| pole.position.x)
        .fold(f32::INFINITY, f32::min);
    assert!(
        (nearest - mouth as f32 - super::DEAD_END_MARGIN_M).abs() < 1e-3,
        "highest priority keeps its normal margin"
    );
}

#[test]
fn six_arm_junctions_keep_poles_apart_and_every_arm_lit() {
    for angles in [
        [0.0_f32, 60.0, 120.0, 180.0, 240.0, 300.0],
        [0.0_f32, 30.0, 60.0, 180.0, 210.0, 240.0],
    ] {
        for lanes in [(1, 1), (2, 2)] {
            let mut points = vec![Vector3::ZERO];
            points.extend(angles.iter().map(|angle| {
                let angle = angle.to_radians();
                Vector3::new(120.0 * angle.cos(), 0.0, 120.0 * angle.sin())
            }));
            // Alternating draw directions also exercise the unchanged canonical side rule.
            let links: Vec<_> = (1..=6)
                .map(|i| {
                    if i % 2 == 0 {
                        (i, 0, lanes.0, lanes.1)
                    } else {
                        (0, i, lanes.0, lanes.1)
                    }
                })
                .collect();
            let f = fixture(&points, &links);
            let arms: Vec<_> = (0..6).map(|edge| vec![edge]).collect();
            assert_arm_clearance_and_partition(&f, &arms);
        }
    }
}

#[test]
fn junction_clearance_and_coverage_follow_whole_chains() {
    let f = fixture(
        &[
            Vector3::ZERO,
            Vector3::new(24.0, 0.0, 0.0),
            Vector3::new(48.0, 0.0, 0.0),
            Vector3::new(120.0, 0.0, 0.0),
            Vector3::new(-120.0, 0.0, 0.0),
            Vector3::new(104.0, 0.0, 60.0),
        ],
        &[
            (0, 1, 1, 1),
            (2, 1, 1, 1),
            (2, 3, 1, 1),
            (4, 0, 2, 2),
            (0, 5, 0, 0),
        ],
    );
    assert_arm_clearance_and_partition(&f, &[vec![0, 1, 2], vec![3], vec![4]]);
}

#[test]
fn exhausted_margins_keep_the_existing_single_middle_light() {
    let f = fixture(&STREET, &[(0, 1, 1, 1)]);
    let sections = &f.1.compiled_sections()[&0];
    let mut a = sections[0].clone();
    let mut b = sections[1].clone();
    a.s_m = 0.0;
    b.s_m = 10.0;
    a.center_xz = glam::DVec2::ZERO;
    b.center_xz = glam::DVec2::new(10.0, 0.0);
    let sections = [a, b];
    let run = super::Run {
        sections: &sections,
        offset_m: 0.0,
        length_m: 10.0,
    };
    let rule = super::rule_for_edge(f.0.edge(0)).unwrap();
    let mut mesh = super::NetworkMeshData::new();
    super::place_along_chain(&mut mesh, &f.2, &[(0, true)], &[run], 0, &rule, 8.0, 2.0);
    assert_eq!(mesh.street_lights.len(), 1);
    assert!((mesh.street_lights[0].position.x - 5.0).abs() < 1e-5);
}
