// SPDX-License-Identifier: GPL-2.0-only

//! Cosmetic sign regressions using isolated graph, lane, and surface fixtures.

use super::*;
use crate::simulation::network::graph::Node;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::render::road::RoadRenderer;
use crate::simulation::network::types::{NodeType, TransitFlags};
use crate::simulation::terrain::render_frame_centre;
use std::collections::{BTreeMap, BTreeSet};

fn fixture(
    points: &[Vector3],
    links: &[(usize, usize, u8, u8)],
) -> (RegionGraph, LaneSystem, RoadbedSystem, TerrainSystem) {
    let nodes = points
        .iter()
        .map(|&pos| Node {
            pos,
            node_type: NodeType::default(),
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
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                class: EdgeClass::Standard,
                width: f32::from(fwd + bkw) * crate::config::LANE_WIDTH,
                fwd_lanes: fwd,
                bkw_lanes: bkw,
                speed_limit: 50.0 / 3.6,
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
    // Disable procedural zebras in the baseline fixture; crossing tests enable one explicitly.
    for (edge, &(start, end, _, _)) in links.iter().enumerate() {
        graph.set_crosswalk_override(start as u32, edge, false);
        graph.set_crosswalk_override(end as u32, edge, false);
    }
    let terrain = TerrainSystem::with_chunking(256, 256, 1.0, 8, 0.0);
    let mut surface = RoadbedSystem::new(64.0);
    surface.compile_dirty(&graph);
    let mut lanes = LaneSystem::new();
    lanes.rebuild(&mut graph);
    (graph, lanes, surface, terrain)
}

fn junction() -> (RegionGraph, LaneSystem, RoadbedSystem, TerrainSystem) {
    junction_with_lanes(1, 1)
}

fn junction_with_lanes(
    fwd: u8,
    bkw: u8,
) -> (RegionGraph, LaneSystem, RoadbedSystem, TerrainSystem) {
    let points = [
        Vector3::ZERO,
        Vector3::new(120.0, 0.0, 0.0),
        Vector3::new(-120.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 120.0),
    ];
    let (mut graph, mut lanes, mut surface, terrain) =
        fixture(&points, &[(0, 1, fwd, bkw), (2, 0, 1, 1), (0, 3, 1, 1)]);
    // Keep one authoritative zebra, on edge 0. No crossing geometry is constructed by the test:
    // the crossing moves the junction's mouth on edge 0, so the roadbed is laid out again.
    graph.set_crosswalk_override(0, 0, true);
    graph.set_crosswalk_override(0, 1, false);
    graph.set_crosswalk_override(0, 2, false);
    surface.clear();
    surface.compile_dirty(&graph);
    lanes.rebuild(&mut graph);
    (graph, lanes, surface, terrain)
}

fn generate(
    f: &(RegionGraph, LaneSystem, RoadbedSystem, TerrainSystem),
) -> super::super::NetworkMeshData {
    RoadRenderer.generate_mesh_data_with_surface(&f.0, &f.3, &f.2)
}

fn crossing(kind: RoadSignKind) -> bool {
    matches!(
        kind,
        RoadSignKind::CrossingRight | RoadSignKind::CrossingLeft
    )
}

fn bytes(signs: &[RoadSign]) -> Vec<u32> {
    signs
        .iter()
        .flat_map(|s| {
            [
                s.position.x.to_bits(),
                s.position.y.to_bits(),
                s.position.z.to_bits(),
                s.yaw.to_bits(),
                s.kind as u32,
            ]
        })
        .collect()
}

#[test]
fn two_way_zebra_has_two_opposite_right_hand_signs_at_the_kerb() {
    let f = junction();
    let mesh = generate(&f);
    let signs: Vec<_> = mesh
        .road_signs
        .iter()
        .filter(|s| crossing(s.kind))
        .collect();
    assert_eq!(signs.len(), 2, "{signs:?}");
    let marking = f.1.node_lanes[&0]
        .iter()
        .find_map(|&lane| f.1.crosswalk_marking(lane))
        .unwrap();
    let center = (marking.start + marking.end) * 0.5;
    for sign in &signs {
        assert_eq!(sign.kind, RoadSignKind::CrossingRight);
        assert!(
            (sign.position.x - center.x).abs() < 2.0,
            "beside zebra: {sign:?}, {center:?}"
        );
        assert!(
            (sign.position.z.abs() - (crate::config::LANE_WIDTH + 0.5)).abs() < 0.01,
            "pole must be 0.5 m behind the kerb face: {sign:?}"
        );
        // The stripes end at the carriageway boundary, inside both pole bases.
        assert!(sign.position.z.abs() > marking.start.z.abs().max(marking.end.z.abs()));
        let front = Vector3::new(sign.yaw.sin(), 0.0, sign.yaw.cos());
        // Just before the stripes for the traffic it faces, out of the walking line.
        let setback = (sign.position.x - center.x) * front.x;
        assert!(
            (1.25..2.0).contains(&setback),
            "sign at the approach edge: {setback}"
        );
        assert!(
            front.x * sign.position.z.signum() < -0.99,
            "face approaching traffic: {sign:?}"
        );
    }
    assert!(signs[0].position.z * signs[1].position.z < 0.0);
    assert!((signs[0].yaw - signs[1].yaw).abs() > 3.0);
}

#[test]
fn pass_through_zebra_signs_stand_beside_its_span_strip() {
    // A straight road split at node 0. Its one crossing (D8) lies on edge 0's span, from the
    // node out to 3 m, and both signs follow that marking. Street lights space along the whole
    // street through degree-two nodes and do not avoid its crossing; here none stands near it.
    let points = [
        Vector3::ZERO,
        Vector3::new(160.0, 0.0, 0.0),
        Vector3::new(-80.0, 0.0, 0.0),
    ];
    let (mut graph, mut lanes, mut surface, terrain) =
        fixture(&points, &[(0, 1, 1, 1), (2, 0, 1, 1)]);
    graph.set_crosswalk_override(0, 0, true);
    surface.clear();
    surface.compile_dirty(&graph);
    lanes.rebuild(&mut graph);
    let f = (graph, lanes, surface, terrain);
    let marking = f.1.node_lanes[&0]
        .iter()
        .find_map(|&lane| f.1.crosswalk_marking(lane))
        .unwrap();
    let center = (marking.start + marking.end) * 0.5;
    assert!((center.x - 1.5).abs() < 1e-5, "{center:?}");
    let mesh = generate(&f);
    let signs: Vec<_> = mesh
        .road_signs
        .iter()
        .filter(|s| crossing(s.kind))
        .collect();
    assert_eq!(signs.len(), 2, "{signs:?}");
    for sign in &signs {
        assert!(
            (sign.position.z.abs() - (crate::config::LANE_WIDTH + 0.5)).abs() < 0.01,
            "pole must be 0.5 m behind the kerb face: {sign:?}"
        );
        let front = Vector3::new(sign.yaw.sin(), 0.0, sign.yaw.cos());
        let setback = (sign.position.x - center.x) * front.x;
        assert!(
            (1.25..2.0).contains(&setback),
            "sign at the approach edge: {setback}"
        );
    }
    assert!(signs[0].position.z * signs[1].position.z < 0.0);
}

#[test]
fn uniform_limits_have_no_speed_signs_and_changed_limit_follows_the_zebra() {
    let mut f = junction();
    assert!(
        generate(&f)
            .road_signs
            .iter()
            .all(|sign| crossing(sign.kind))
    );
    f.0.edge_mut(0).speed_limit = 30.0 / 3.6;
    let mesh = generate(&f);
    let speeds: Vec<_> = mesh
        .road_signs
        .iter()
        .filter(|s| s.kind == RoadSignKind::Speed30)
        .collect();
    assert_eq!(
        speeds.len(),
        1,
        "only traffic entering the 30 street: {speeds:?}"
    );
    let speed = speeds[0];
    let zebra = mesh
        .road_signs
        .iter()
        .find(|s| crossing(s.kind) && s.position.z > 0.0)
        .unwrap();
    assert!(speed.position.x >= zebra.position.x + SPEED_AFTER_CROSSING_M - 1e-4);
    assert!(speed.yaw.sin() < -0.99);
    assert_eq!(
        mesh.road_signs
            .iter()
            .filter(|s| s.kind == RoadSignKind::Speed50)
            .count(),
        2
    );
    assert!(bytes(&mesh.road_signs) == bytes(&generate(&f).road_signs));
}

#[test]
fn a_long_street_repeats_the_uniform_limit_after_its_junction() {
    let points = [
        Vector3::ZERO,
        Vector3::new(SPEED_REPEAT_STREET_M + 20.0, 0.0, 0.0),
        Vector3::new(-120.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 120.0),
    ];
    let (mut graph, mut lanes, mut surface, terrain) =
        fixture(&points, &[(0, 1, 1, 1), (2, 0, 1, 1), (0, 3, 1, 1)]);
    graph.set_crosswalk_override(0, 0, true);
    // Signs stand at the pieces' crossings: lay the junction out again with its zebra.
    surface.clear();
    surface.compile_dirty(&graph);
    lanes.rebuild(&mut graph);
    let mesh = generate(&(graph, lanes, surface, terrain));
    let speeds: Vec<_> = mesh
        .road_signs
        .iter()
        .filter(|s| !crossing(s.kind))
        .collect();
    // Only traffic turning into the long street; the 120 m side streets stay unsigned.
    assert_eq!(speeds.len(), 1, "{speeds:?}");
    assert_eq!(speeds[0].kind, RoadSignKind::Speed50);
    let zebra = mesh
        .road_signs
        .iter()
        .find(|s| crossing(s.kind) && s.position.z > 0.0)
        .unwrap();
    assert!(speeds[0].position.x >= zebra.position.x + SPEED_AFTER_CROSSING_M - 1e-4);
}

#[test]
fn speed_changes_compare_mapped_live_road_limits() {
    let mut f = junction();
    f.0.edge_mut(0).speed_limit = 54.0 / 3.6;
    assert!(generate(&f).road_signs.iter().all(|s| crossing(s.kind)));
    f.0.edge_mut(0).speed_limit = 30.0 / 3.6;
    f.0.edge_mut(2).deleted = true;
    assert!(generate(&f).road_signs.iter().all(|s| crossing(s.kind)));
}

#[test]
fn short_street_dead_ends_and_degree_two_nodes_have_no_speed_signs() {
    let mut f = junction();
    f.0.edge_mut(0).speed_limit = 30.0 / 3.6;
    f.0.edge_mut(0).physical_length = 30.0;
    assert!(
        !generate(&f)
            .road_signs
            .iter()
            .any(|s| s.kind == RoadSignKind::Speed30)
    );
    let mut f = fixture(
        &[
            Vector3::ZERO,
            Vector3::new(24.0, 0.0, 0.0),
            Vector3::new(48.0, 0.0, 0.0),
            Vector3::new(72.0, 0.0, 0.0),
        ],
        &[(0, 1, 1, 1), (1, 2, 1, 1), (2, 3, 1, 1)],
    );
    f.0.edge_mut(1).speed_limit = 30.0 / 3.6;
    assert!(generate(&f).road_signs.is_empty());
    let f = fixture(
        &[Vector3::ZERO, Vector3::new(120.0, 0.0, 0.0)],
        &[(0, 1, 1, 0)],
    );
    assert!(generate(&f).road_signs.is_empty());
}

#[test]
fn signs_keep_two_metres_from_every_light_and_shift_away_from_junction() {
    let f = junction();
    let mesh = generate(&f);
    for sign in &mesh.road_signs {
        for pole in &mesh.street_lights {
            assert!(
                (sign.position.x - pole.position.x).hypot(sign.position.z - pole.position.z) >= 2.0,
                "{sign:?} near {pole:?}"
            );
        }
    }
    // Force an exact conflict to exercise shifting, rather than only observing normal spacing.
    let sign = mesh
        .road_signs
        .iter()
        .find(|s| crossing(s.kind) && s.position.z > 0.0)
        .unwrap();
    let mut poles = PoleGrid {
        heads: HashMap::new(),
        poles: Vec::new(),
    };
    poles.heads.insert(PoleGrid::cell(sign.position), 0);
    poles.poles.push((sign.position, NO_LINK));
    // The zebra lies on the node piece: place on the span extended to the crosswalk.
    let sections = crossing_sections(&f.0, &f.2, 0, f.2.sections(0).unwrap());
    let marking = f.1.node_lanes[&0]
        .iter()
        .find_map(|&lane| f.1.crosswalk_marking(lane))
        .unwrap();
    let s = crossing_s(sections, marking).unwrap();
    let (shifted, shifted_s) = place(
        sections,
        &f.3,
        &poles,
        s,
        1.0,
        1.0,
        1.0,
        RoadSignKind::CrossingRight,
    )
    .unwrap();
    assert!(shifted_s > s && shifted.position.x > sign.position.x);
    assert!(shifted.position.distance_to(sign.position) >= 2.0);
}

#[test]
fn speed_mapping_is_bounded_and_ties_choose_lower_value() {
    for (kmh, expected) in [
        (30.0, Some(RoadSignKind::Speed30)),
        (35.0, Some(RoadSignKind::Speed30)),
        (44.0, Some(RoadSignKind::Speed40)),
        (60.0, Some(RoadSignKind::Speed60)),
        (80.0, Some(RoadSignKind::Speed80)),
        (70.0, None),
        (100.0, None),
    ] {
        assert_eq!(speed_kind(kmh / 3.6), expected, "{kmh}");
    }
    assert_eq!(speed_kind(f32::NAN), None);
}

#[test]
fn repeated_output_is_byte_identical_and_chunk_replacements_match_full_mesh() {
    let mut f = junction();
    f.0.edge_mut(0).speed_limit = 30.0 / 3.6;
    assert_chunk_output(&mut f);
}

fn assert_chunk_output(f: &mut (RegionGraph, LaneSystem, RoadbedSystem, TerrainSystem)) {
    let first = generate(f);
    assert_eq!(bytes(&first.road_signs), bytes(&generate(f).road_signs));
    let chunks: BTreeSet<_> = (-3..=3)
        .flat_map(|x| (-3..=3).map(move |z| (x, z)))
        .collect();
    let partitioned = RoadRenderer.generate_mesh_chunks_with_surface(
        &f.0,
        &mut f.1,
        &f.3,
        &crate::simulation::terrain::graded::NoSites,
        &f.2,
        &chunks,
        None,
        &BTreeMap::new(),
    );
    let (ox, oz) = f.2.chunk_origin_m();
    let mut restored = Vec::new();
    for (&(x, z), mesh) in &partitioned {
        assert!(mesh.render_payload_is_valid());
        for sign in &mesh.road_signs {
            let mut sign = *sign;
            let span = f.2.chunk_span_m();
            sign.position.x += render_frame_centre(ox + x as f32 * span, span);
            sign.position.z += render_frame_centre(oz + z as f32 * span, span);
            restored.push(sign);
        }
    }
    order_signs(&mut restored);
    assert_eq!(restored.len(), first.road_signs.len());
    for (a, b) in restored.iter().zip(&first.road_signs) {
        assert_eq!(a.kind, b.kind);
        assert!((a.position - b.position).length() < 1e-4);
        assert_eq!(a.yaw.to_bits(), b.yaw.to_bits());
    }
    for (&chunk, expected) in &partitioned {
        let rebuilt = RoadRenderer.generate_mesh_chunks_with_surface(
            &f.0,
            &mut f.1,
            &f.3,
            &crate::simulation::terrain::graded::NoSites,
            &f.2,
            &BTreeSet::from([chunk]),
            None,
            &BTreeMap::new(),
        );
        let actual = rebuilt
            .get(&chunk)
            .map(|mesh| bytes(&mesh.road_signs))
            .unwrap_or_default();
        assert_eq!(
            actual,
            bytes(&expected.road_signs),
            "individual chunk {chunk:?}"
        );
    }
}

#[test]
fn one_way_zebra_uses_the_left_variant_facing_the_same_approach() {
    let f = junction_with_lanes(1, 0);
    let mesh = generate(&f);
    let signs: Vec<_> = mesh
        .road_signs
        .iter()
        .filter(|s| crossing(s.kind))
        .collect();
    assert_eq!(signs.len(), 2);
    assert!(
        signs
            .iter()
            .any(|s| s.kind == RoadSignKind::CrossingLeft && s.position.z < 0.0)
    );
    assert!(
        signs
            .iter()
            .any(|s| s.kind == RoadSignKind::CrossingRight && s.position.z > 0.0)
    );
    assert!(signs.iter().all(|s| s.yaw.sin() < -0.99));
}

#[test]
fn every_sign_pole_uses_the_kerb_face_and_interpolated_sidewalk_height() {
    let f = junction();
    let mut sections = f.2.sections(0).unwrap().to_vec();
    for section in &mut sections {
        for band in &mut section.bands {
            if band.kind == BandKind::Sidewalk {
                band.height_start_m = 1.0;
                band.height_end_m = 2.0;
            }
        }
    }
    let section = &sections[sections.len() / 2];
    for side in [1.0, -1.0] {
        let sidewalk = section
            .bands
            .iter()
            .find(|band| {
                band.kind == BandKind::Sidewalk
                    && if side > 0.0 {
                        band.lateral_end_m > 0.0
                    } else {
                        band.lateral_start_m < 0.0
                    }
            })
            .unwrap();
        let (near, far, h0, h1) = if side > 0.0 {
            (
                sidewalk.lateral_start_m,
                sidewalk.lateral_end_m,
                sidewalk.height_start_m,
                sidewalk.height_end_m,
            )
        } else {
            (
                -sidewalk.lateral_end_m,
                -sidewalk.lateral_start_m,
                sidewalk.height_end_m,
                sidewalk.height_start_m,
            )
        };
        let (distance, height) = offset(section, side).unwrap();
        assert!((distance - (crate::config::LANE_WIDTH + 0.5)).abs() < 1e-5);
        assert!(distance > near && distance < far);
        assert!(
            (height.unwrap() - (h0 + (h1 - h0) * (distance - near) / (far - near))).abs() < 1e-5
        );
        for kind in [
            RoadSignKind::CrossingRight,
            RoadSignKind::CrossingLeft,
            RoadSignKind::Speed50,
            RoadSignKind::CurveRight,
            RoadSignKind::Uneven,
        ] {
            let sign = pose(SectionRun::span(&sections), &f.3, 60.0, side, side, kind).unwrap();
            assert!((sign.position.z.abs() - distance).abs() < 1e-5);
            assert_eq!(sign.position.y, height.unwrap());
        }
    }
}

#[test]
fn without_a_sidewalk_the_sign_uses_the_verge_and_terrain_height() {
    let f = fixture(
        &[Vector3::ZERO, Vector3::new(120.0, 0.0, 0.0)],
        &[(0, 1, 1, 1)],
    );
    let mut sections = f.2.sections(0).unwrap().to_vec();
    for section in &mut sections {
        section.bands.retain(|band| band.kind != BandKind::Sidewalk);
    }
    for side in [1.0, -1.0] {
        let (distance, height) = offset(&sections[sections.len() / 2], side).unwrap();
        assert!(height.is_none());
        let outer = sections[sections.len() / 2]
            .bands
            .iter()
            .filter(|band| matches!(band.kind, BandKind::CurbOrShoulder | BandKind::Carriageway))
            .map(|band| {
                if side > 0.0 {
                    band.lateral_end_m
                } else {
                    -band.lateral_start_m
                }
            })
            .max_by(f32::total_cmp)
            .unwrap();
        assert_eq!(distance, outer + 0.75);
        let sign = pose(
            SectionRun::span(&sections),
            &f.3,
            60.0,
            side,
            side,
            RoadSignKind::Speed50,
        )
        .unwrap();
        assert_eq!(
            sign.position.y,
            f.3.sample_height_world(sign.position.x, sign.position.z)
        );
    }
}

// Preserve the supplied physical profile: full gameplay/index rebuilds can engineer it away.
fn profile(
    points: &[Vector3],
    fwd: u8,
    bkw: u8,
) -> (RegionGraph, LaneSystem, RoadbedSystem, TerrainSystem) {
    let mut f = fixture(&[points[0], *points.last().unwrap()], &[(0, 1, fwd, bkw)]);
    let edge = f.0.edge_mut(0);
    edge.physical_geometry = points.to_vec();
    edge.geometry = points.to_vec();
    edge.physical_length = points.windows(2).map(|p| p[0].distance_to(p[1])).sum();
    f.2.mark_edge_dirty(0);
    f.2.compile_dirty(&f.0);
    f
}

fn warning_kinds(f: &(RegionGraph, LaneSystem, RoadbedSystem, TerrainSystem)) -> Vec<RoadSignKind> {
    generate(f)
        .road_signs
        .into_iter()
        .filter(|s| s.kind as u8 >= RoadSignKind::CurveRight as u8)
        .map(|s| s.kind)
        .collect()
}

#[test]
fn a_short_steep_ramp_on_a_gentle_street_gets_no_steep_sign() {
    // 8 m at 25 % at the junction end of a flat 100 m street: the junction tie-in bump.
    let f = profile(
        &[
            Vector3::ZERO,
            Vector3::new(8.0, 2.0, 0.0),
            Vector3::new(108.0, 2.0, 0.0),
        ],
        1,
        1,
    );
    assert!(
        warning_kinds(&f)
            .iter()
            .all(|&kind| kind == RoadSignKind::Uneven)
    );
}

#[test]
fn steep_stretch_uses_signed_direction_and_peak_even_percentage() {
    let f = profile(
        &[
            Vector3::ZERO,
            Vector3::new(30.0, 0.0, 0.0),
            Vector3::new(50.0, 3.4, 0.0),
            Vector3::new(160.0, 22.1, 0.0),
        ],
        1,
        1,
    );
    let kinds = warning_kinds(&f);
    assert_eq!(kinds.len(), 2, "{kinds:?}");
    assert!(kinds.contains(&RoadSignKind::Ascent16));
    assert!(kinds.contains(&RoadSignKind::Descent16));
    let signs = generate(&f).road_signs;
    assert!(
        signs
            .iter()
            .any(|s| s.kind == RoadSignKind::Ascent16 && s.yaw.sin() < -0.99)
    );
    assert!(
        signs
            .iter()
            .any(|s| s.kind == RoadSignKind::Descent16 && s.yaw.sin() > 0.99)
    );
    let gentle = profile(&[Vector3::ZERO, Vector3::new(120.0, 6.0, 0.0)], 1, 1);
    assert!(warning_kinds(&gentle).is_empty());
    let short = profile(&[Vector3::ZERO, Vector3::new(10.0, 1.5, 0.0)], 1, 1);
    assert!(warning_kinds(&short).is_empty());
    assert_eq!(steep_kind(0.099), RoadSignKind::Ascent8);
    assert_eq!(steep_kind(-0.30), RoadSignKind::Descent18);
}

fn arc(points: &mut Vec<Vector3>, heading: &mut f32, hand: f32) {
    for _ in 0..18 {
        *heading += hand * 5.0f32.to_radians();
        let last = *points.last().unwrap();
        points.push(last + Vector3::new(heading.cos(), 0.0, heading.sin()) * 2.0);
    }
}

#[test]
fn tight_curve_and_s_curve_use_driver_hand() {
    let mut points = vec![Vector3::ZERO, Vector3::new(40.0, 0.0, 0.0)];
    let mut heading = 0.0;
    arc(&mut points, &mut heading, 1.0);
    points.push(*points.last().unwrap() + Vector3::new(0.0, 0.0, 40.0));
    let f = profile(&points, 1, 1);
    let kinds = warning_kinds(&f);
    assert_eq!(kinds.len(), 2, "{kinds:?}");
    assert!(kinds.contains(&RoadSignKind::CurveRight));
    assert!(kinds.contains(&RoadSignKind::CurveLeft));
    points.pop();
    points.push(*points.last().unwrap() + Vector3::new(0.0, 0.0, 20.0));
    arc(&mut points, &mut heading, -1.0);
    points.push(*points.last().unwrap() + Vector3::new(40.0, 0.0, 0.0));
    let f = profile(&points, 1, 1);
    let kinds = warning_kinds(&f);
    assert_eq!(kinds, vec![RoadSignKind::CurvesRight; 2]);
    let mut chunk_fixture = profile(&points, 1, 1);
    assert_chunk_output(&mut chunk_fixture);
    let one_way = profile(&points, 1, 0);
    assert_eq!(warning_kinds(&one_way), vec![RoadSignKind::CurvesRight]);
    assert_eq!(
        bytes(&generate(&f).road_signs),
        bytes(&generate(&f).road_signs)
    );
    // A2 requires two qualifying bends; a 30-degree second turn leaves only the first A1.
    points.truncate(27);
    let tangent = Vector3::new(
        60.0_f32.to_radians().cos(),
        0.0,
        60.0_f32.to_radians().sin(),
    );
    points.push(*points.last().unwrap() + tangent * 40.0);
    assert_eq!(
        warning_kinds(&profile(&points, 1, 0)),
        vec![RoadSignKind::CurveRight]
    );
}

#[test]
fn ordinary_radius_residential_speed_and_shallow_turn_get_no_curve_warning() {
    let mut points = vec![Vector3::ZERO, Vector3::new(40.0, 0.0, 0.0)];
    let mut heading = 0.0;
    arc(&mut points, &mut heading, 1.0);
    points.push(*points.last().unwrap() + Vector3::new(0.0, 0.0, 40.0));
    let mut residential = profile(&points, 1, 1);
    residential.0.edge_mut(0).speed_limit = 30.0 / 3.6;
    assert!(warning_kinds(&residential).is_empty());
    let ordinary: Vec<_> = points.iter().map(|point| *point * 1.5).collect();
    assert!(warning_kinds(&profile(&ordinary, 1, 1)).is_empty());
    points.truncate(12); // Ten five-degree steps: a sharp but unsurprising 50-degree turn.
    points.push(
        *points.last().unwrap()
            + Vector3::new(
                50.0_f32.to_radians().cos(),
                0.0,
                50.0_f32.to_radians().sin(),
            ) * 40.0,
    );
    assert!(warning_kinds(&profile(&points, 1, 1)).is_empty());
}

#[test]
fn curve_near_a_junction_mouth_is_suppressed_in_both_directions() {
    for tail in [20.0, 45.0, 80.0] {
        let mut points = vec![Vector3::ZERO, Vector3::new(40.0, 0.0, 0.0)];
        let mut heading = 0.0;
        arc(&mut points, &mut heading, 1.0);
        points.push(*points.last().unwrap() + Vector3::new(0.0, 0.0, tail));
        let end = *points.last().unwrap();
        let mut f = fixture(
            &[
                points[0],
                end,
                end + Vector3::new(80.0, 0.0, 0.0),
                end - Vector3::new(80.0, 0.0, 0.0),
            ],
            &[(0, 1, 1, 1), (1, 2, 1, 1), (3, 1, 1, 1)],
        );
        let edge = f.0.edge_mut(0);
        edge.physical_geometry = points.clone();
        edge.geometry = points.clone();
        edge.physical_length = points
            .windows(2)
            .map(|pair| pair[0].distance_to(pair[1]))
            .sum();
        edge.end_clip = 20.0;
        let warnings = street_lengths(&f.0, &[0], &mut HashMap::new()).warnings;
        let curves = warnings
            .values()
            .flat_map(|families| [families[0], families[3]])
            .flatten()
            .count();
        assert_eq!(
            curves,
            if tail > 50.0 { 2 } else { 0 },
            "tail={tail}, {warnings:?}"
        );
        // Stored edge reversal must preserve mouth exclusion and driver-facing warnings.
        let edge = f.0.edge_mut(0);
        std::mem::swap(&mut edge.start_node, &mut edge.end_node);
        std::mem::swap(&mut edge.start_clip, &mut edge.end_clip);
        edge.physical_geometry.reverse();
        edge.geometry.reverse();
        assert_eq!(
            street_lengths(&f.0, &[0], &mut HashMap::new())
                .warnings
                .values()
                .flat_map(|families| [families[0], families[3]])
                .flatten()
                .count(),
            curves
        );
    }
}

#[test]
fn degree_two_node_fillets_are_not_measured_as_road_bends() {
    let mut f = fixture(
        &[
            Vector3::ZERO,
            Vector3::new(100.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 100.0),
        ],
        &[(0, 1, 1, 1), (1, 2, 1, 1)],
    );
    for id in 0..2 {
        let edge = f.0.edge_mut(id);
        let (start, end) = (edge.physical_geometry[0], edge.physical_geometry[1]);
        edge.physical_geometry = (0..=20)
            .map(|step| start.lerp(end, step as f32 / 20.0))
            .collect();
    }
    let curves = |graph: &RegionGraph| {
        street_lengths(graph, &[0], &mut HashMap::new())
            .warnings
            .values()
            .flat_map(|families| [families[0], families[3]])
            .flatten()
            .count()
    };
    assert_eq!(curves(&f.0), 2); // The untrimmed physical polyline contains a sharp corner.
    // Isolated fixtures do not solve node fillets; supply their authoritative mouth clips.
    f.0.edge_mut(0).end_clip = 5.5;
    f.0.edge_mut(1).start_clip = 5.5;
    assert_eq!(curves(&f.0), 0);
    let edge = f.0.edge_mut(1);
    std::mem::swap(&mut edge.start_node, &mut edge.end_node);
    std::mem::swap(&mut edge.start_clip, &mut edge.end_clip);
    edge.physical_geometry.reverse();
    assert_eq!(curves(&f.0), 0);
}

#[test]
fn shared_radius_windows_report_complete_travelled_station_extents() {
    let points = [
        Vector3::ZERO,
        Vector3::new(4.0, 3.0, 0.0),
        Vector3::new(4.0, 3.0, 4.0),
        Vector3::new(8.0, 6.0, 4.0),
    ];
    let mut samples = Vec::new();
    visit_turning_radii(
        &points,
        |_| 4.0,
        |station, _, _, measured| samples.push((station, measured)),
    );
    assert_eq!(samples, [(5.0, Some((2.5, 7.0))), (9.0, Some((7.0, 11.5)))]);
    visit_turning_radii(
        &points,
        |_| 100.0,
        |_, radius, _, measured| {
            assert!(radius.is_finite()); // Placement validation still measures partial windows.
            assert!(measured.is_none());
        },
    );
}

#[test]
fn a_degree_two_chain_too_short_for_the_radius_window_has_no_curve_warning() {
    let mut points = vec![Vector3::ZERO];
    let mut heading = 0.0;
    arc(&mut points, &mut heading, 1.0);
    for point in &mut points {
        *point *= 0.2;
    }
    let links: Vec<_> = (0..points.len() - 1)
        .map(|index| (index, index + 1, 1, 1))
        .collect();
    let f = fixture(&points, &links);
    let warnings = street_lengths(&f.0, &[0], &mut HashMap::new()).warnings;
    assert!(
        warnings
            .values()
            .all(|families| families[0].is_none() && families[3].is_none()),
        "{warnings:?}"
    );
}

#[test]
fn uneven_and_steep_signs_obey_gap_and_pole_clearance() {
    let f = profile(
        &[
            Vector3::ZERO,
            Vector3::new(30.0, 0.0, 0.0),
            Vector3::new(32.0, 0.0, 0.0),
            Vector3::new(34.0, 0.4, 0.0),
            Vector3::new(140.0, 21.6, 0.0),
        ],
        1,
        1,
    );
    let mesh = generate(&f);
    assert_eq!(mesh.road_signs.len(), 4, "{:?}", mesh.road_signs);
    assert_eq!(
        mesh.road_signs
            .iter()
            .filter(|s| s.kind == RoadSignKind::Uneven)
            .count(),
        2
    );
    let sections = f.2.sections(0).unwrap();
    for (i, a) in mesh.road_signs.iter().enumerate() {
        for b in &mesh.road_signs[i + 1..] {
            if a.position.z.signum() == b.position.z.signum() {
                let station = |sign: &RoadSign| {
                    crossing_s(
                        SectionRun::span(sections),
                        CrosswalkMarking {
                            start: sign.position,
                            end: sign.position,
                            edge_id: 0,
                        },
                    )
                    .unwrap()
                };
                assert!(
                    (station(a) - station(b)).abs() >= SIGN_GAP_M - 1e-4,
                    "{a:?}, {b:?}"
                );
            }
        }
        for pole in &mesh.street_lights {
            assert!(a.position.distance_to(pole.position) >= CLEARANCE_M);
        }
    }
    assert_eq!(bytes(&mesh.road_signs), bytes(&generate(&f).road_signs));
    let poles = PoleGrid {
        heads: HashMap::new(),
        poles: Vec::new(),
    };
    let end = sections.last().unwrap().s_m;
    let span = ChainSpan {
        chain_id: 0,
        edge_id: 0,
        start: 0.0,
        end,
        forward: true,
        loop_length: None,
    };
    let mut spacing = SignSpacing::new(1);
    spacing.insert(&span, end - 2.0, 1.0, 1.0);
    assert!(
        place_spaced(
            sections,
            &f.3,
            &poles,
            end - 1.0,
            1.0,
            RoadSignKind::Uneven,
            &spacing,
            &span
        )
        .is_none()
    );
}

#[test]
fn degree_two_nodes_share_steep_hazard_and_total_stretch_length() {
    let mut f = fixture(
        &[
            Vector3::ZERO,
            Vector3::new(40.0, 6.8, 0.0),
            Vector3::new(80.0, 13.6, 0.0),
            Vector3::new(120.0, 20.4, 0.0),
        ],
        &[(0, 1, 1, 1), (2, 1, 1, 1), (2, 3, 1, 1)],
    );
    for (id, (a, b)) in [(0, (0, 1)), (1, (2, 1)), (2, (2, 3))] {
        let points = [
            Vector3::new(a as f32 * 40.0, a as f32 * 6.8, 0.0),
            Vector3::new(b as f32 * 40.0, b as f32 * 6.8, 0.0),
        ];
        f.0.edge_mut(id).physical_geometry = points.to_vec();
        f.0.edge_mut(id).physical_length = points[0].distance_to(points[1]);
        f.2.mark_edge_dirty(id);
    }
    f.2.compile_dirty(&f.0);
    let kinds = warning_kinds(&f);
    assert_eq!(kinds.len(), 2, "{kinds:?}");
    assert!(kinds.contains(&RoadSignKind::Ascent16));
    assert!(kinds.contains(&RoadSignKind::Descent16));
    let mut degrees = HashMap::new();
    let a = street_lengths(&f.0, &[0], &mut degrees).warnings;
    let b = street_lengths(&f.0, &[2, 1, 0], &mut HashMap::new()).warnings;
    assert_eq!(a, b);
}

#[test]
fn spacing_crosses_reversed_edge_and_loop_seam_but_respects_facing() {
    let a = ChainSpan {
        chain_id: 0,
        edge_id: 0,
        start: 0.0,
        end: 40.0,
        forward: true,
        loop_length: None,
    };
    let b = ChainSpan {
        chain_id: 0,
        edge_id: 1,
        start: 40.0,
        end: 80.0,
        forward: false,
        loop_length: None,
    };
    let mut spacing = SignSpacing::new(4);
    spacing.insert(&a, 37.0, 1.0, 1.0);
    assert_eq!(spacing.advance(&b, 37.0, -1.0, -1.0), 6.0);
    assert_eq!(spacing.advance(&b, 31.0, -1.0, -1.0), 0.0);
    assert_eq!(spacing.advance(&b, 37.0, 1.0, 1.0), 0.0);
    let loop_span = ChainSpan {
        chain_id: 1,
        loop_length: Some(80.0),
        ..a
    };
    spacing.insert(&loop_span, 77.0, 1.0, 1.0);
    assert_eq!(spacing.advance(&loop_span, 3.0, 1.0, 1.0), 6.0);
}
