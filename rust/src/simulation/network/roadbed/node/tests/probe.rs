// SPDX-License-Identifier: GPL-2.0-only

//! The roadbed product probe over every roadbed node and span test fixture, flat and sloped:
//! node and span pieces, layouts, walkways and lanes hash exactly as recorded. Refactors that
//! claim no behaviour change must keep these values; an intended change re-records them from
//! the failure message.

use super::*;
use crate::simulation::network::roadbed::StripKind;
use crate::simulation::network::roadbed::fixtures::{
    ProductProbe, assert_probes, assert_strip_probes, bend, footpath_fixtures, mixed_profiles,
    t_junction,
};
use crate::simulation::network::types::EdgeClass;

// Two junctions joined by a short link, as `neighbouring_junctions_on_a_short_edge_fit_or_report_
// corner_does_not_fit` builds them: some fit, some report `CornerDoesNotFit`.
fn short_link(link_m: f32) -> RegionGraph {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(point(0.0, 0.0, true), NodeType::Junction);
    let east = graph.add_node(point(link_m, 0.0, true), NodeType::Junction);
    connect(&mut graph, west, east, TWO_LANE_M, true);
    for (node, x) in [(west, 0.0), (east, link_m)] {
        for dz in [ARM_M, -ARM_M] {
            let far = graph.add_node(point(x, dz, true), NodeType::Junction);
            connect(&mut graph, node, far, TWO_LANE_M, true);
        }
    }
    graph.rebuild_intersection_clips();
    graph
}

fn probe_fixtures() -> Vec<(String, RegionGraph)> {
    let mut set = Vec::new();
    for sloped in [false, true] {
        set.extend(
            fixtures(sloped)
                .into_iter()
                .map(|(name, graph)| (format!("{name}/{sloped}"), graph)),
        );
        set.extend(footpath_fixtures(sloped));
        for (name, angles) in [
            ("pass_through_north_south", [90.0, 270.0]),
            ("pass_through_south_north", [270.0, 90.0]),
            ("pass_through_bent", [80.0, 252.0]),
        ] {
            set.push((
                format!("{name}/{sloped}"),
                star((12.5, -7.25), &equal(&angles), sloped),
            ));
        }
        set.push((format!("t_junction/{sloped}"), t_junction(sloped)));
        set.push((format!("mixed_profiles/{sloped}"), mixed_profiles(sloped)));
        set.push((format!("bend_15_120/{sloped}"), bend(15.0, 120.0, sloped)));
        set.extend(super::families::family_fixtures(sloped));
    }
    for link_m in [10.0, 13.0, 16.0, 20.0, 25.0, 30.0, 40.0] {
        set.push((format!("short_link_{link_m}"), short_link(link_m)));
    }
    set
}

#[test]
fn roadbed_fixture_products_match_the_probe() {
    let actual: Vec<_> = probe_fixtures()
        .into_iter()
        .map(|(name, graph)| (name, ProductProbe::of_graph(&graph)))
        .collect();
    assert_probes(&actual, EXPECTED);
    assert_strip_probes(&actual, EXPECTED_STRIPS);
}

// `TERRAIN-07-B`: every piece hangs its strips from its own outer chains. Node pieces and
// standard spans stand on the terrain and hang skirts, bridges hang fascias and tunnels none.
// Two triangles per chain segment; their top points are the chain's positions bitwise, their
// bottoms the same points lowered by the strip's depth, and each faces away from the road (the
// right of the chain). Then, on a structure's span with a free end only (`TERRAIN-07-C`), the
// cap: top points of its end rows, bottoms below their corners at the cap kind's depth, facing
// out of the span.
#[test]
fn strips_hang_from_the_outer_chains_bitwise() {
    let mut triangles = Vec::new();
    for (name, graph) in probe_fixtures() {
        let roadbed = compiled(&graph);
        let world = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
        for (id, _) in roadbed.pieces_in_bounds(world) {
            let (mesh, kind) = match id {
                PieceId::Span(edge) => (
                    roadbed.span_piece(edge).unwrap().mesh(),
                    match graph.edge(edge).class {
                        EdgeClass::Standard => Some(StripKind::Skirt),
                        EdgeClass::Bridge => Some(StripKind::Fascia),
                        EdgeClass::Tunnel => None,
                    },
                ),
                PieceId::Node(node) => (
                    roadbed.node_piece(node).unwrap().mesh(),
                    Some(StripKind::Skirt),
                ),
            };
            assert_eq!(mesh.strip_kind(), kind, "{name} {id:?}");
            triangles.clear();
            mesh.for_each_strip_triangle(|points, bottoms| triangles.push((points, bottoms)));
            let mut strip = triangles.iter();
            let chains = if kind.is_some() {
                mesh.outer_chains()
            } else {
                &[]
            };
            for chain in chains {
                let kind = kind.unwrap();
                for pair in chain.vertices().windows(2) {
                    let [a, b] = [pair[0], pair[1]].map(|v| mesh.positions()[v as usize]);
                    let below = |p: RoadVec3| RoadVec3::new(p.x, p.y - kind.depth_m(), p.z);
                    let expected = [
                        ([a, below(a), below(b)], [false, true, true]),
                        ([a, below(b), b], [false, true, false]),
                    ];
                    for (points, bottoms) in expected {
                        let (actual, flags) = strip.next().expect("two triangles per segment");
                        assert_eq!(actual.map(bits), points.map(bits), "{name} {id:?}");
                        assert_eq!(*flags, bottoms, "{name} {id:?}");
                        assert_eq!(mesh.strip_bottom(pair[0]).map(bits), Some(bits(below(a))));
                        // The visible normal `-(q - p) x (r - p)` lies right of the chain.
                        let [p, q, r] = points;
                        let normal = -(q - p).cross(r - p);
                        let right = RoadVec3::new(b.z - a.z, 0.0, a.x - b.x);
                        let length = right.length();
                        assert!(
                            length == 0.0 || normal.dot(right) > 0.0,
                            "{name} {id:?}: a strip faces the road"
                        );
                    }
                }
            }
            let caps: Vec<_> = strip.collect();
            let PieceId::Span(edge) = id else {
                assert!(caps.is_empty(), "{name} {id:?}: one strip per chain");
                continue;
            };
            let span = roadbed.span_piece(edge).unwrap();
            assert_eq!(caps.is_empty(), mesh.cap_kind().is_none(), "{name} {id:?}");
            let Some(cap) = mesh.cap_kind() else {
                continue;
            };
            assert_ne!(graph.edge(edge).class, EdgeClass::Standard, "{name} {id:?}");
            for &(points, bottoms) in caps {
                for (point, bottom) in points.iter().zip(bottoms) {
                    // On an end row, or below one of its corners.
                    let on_row = |mouth: &MouthSection| {
                        let row = &mouth.vertices;
                        let corners = [row[0], row[row.len() - 1]];
                        if bottom {
                            corners.iter().any(|&c| bits(cap.bottom(c)) == bits(*point))
                        } else {
                            row.iter().any(|&p| bits(p) == bits(*point))
                        }
                    };
                    assert!(
                        on_row(span.start_mouth()) || on_row(span.end_mouth()),
                        "{name} {id:?}: a cap vertex off the end rows"
                    );
                }
                // Out of the span: away from the centroid of its vertices.
                let [p, q, r] = points;
                let normal = -(q - p).cross(r - p);
                let positions = mesh.positions();
                let centroid = positions.iter().copied().sum::<RoadVec3>() / positions.len() as f64;
                assert!(
                    normal.dot((p + q + r) / 3.0 - centroid) > 0.0,
                    "{name} {id:?}: a cap faces into the span"
                );
            }
        }
    }
}

// Recorded at `79e7a476` (probe v2: errors by variant and values, layouts field by field) and
// re-recorded by `ROAD-68-D`: every span hash changes with the span outer chains, which now keep the
// road on their left; nodes, layouts, walkways and queries change only where an arm's knot lateral
// differs now that it divides by the segment length instead of multiplying by its reciprocal.
// Re-recorded by `ROAD-68-D` part 3, each change attributed by a build without the other:
// the constructive span station rule changes the spans and queries of every fixture and nothing
// else; the exact terminal cap apex then changes the nodes and layouts of the 65 fixtures whose
// caps have an apex sample (with their walkways, lanes and queries where those follow the cap),
// never the spans. No change touches the graph.
// Re-recorded by `ROAD-68-F` part 2: the road family fixtures are new entries (roads without
// sidewalks, band-only and curved D4 tapers, bridges and tunnels at junctions, bends and
// pass-throughs, and straight degree-two profile changes, which have layouts now), and the
// minimum run beyond a corner changes `path_t` alone (its stem has two concave corners and no
// crossing, so its mouth moves 1 m beyond the farther one; graph clips and everything after them
// follow). The pass-through rule by profile and the removed span taper change no other entry.
// Re-recorded by `ROAD-68-E` part 2 round 4 (rebased onto `ROAD-68-F` part 2), attributed by a
// build with open question 31's sample reverted, which gives part 2's table exactly: the plane
// sample at the mouth station where it lies beyond 12 m changes six of part 2's family entries,
// the sloped `rural_y_40`, `band_width_step`, `band_width_step_curved` and `width_pass_through`,
// and `width_bridge_pass_through` flat and sloped (its bridge climbs 6 m, so its node plane is
// tilted on flat ground too): graph, spans, lanes and queries, and nodes and walkways except for
// `band_width_step` (and walkways for `rural_y_40`, which has none), never the layouts. Their
// mouths lie beyond 12 m (the narrow Y, the D4 tapers). No other entry changes.
const EXPECTED: &[(&str, [u64; 7])] = &[
    (
        "t/false",
        [
            0x3f2ce5378110f69f,
            0xd2c253f19b1ef6da,
            0xacdced0eba6074f4,
            0x99ed70f1e1d83e4e,
            0xb69651d1d18119a9,
            0xf421e77d97d0c086,
            0x62a42ff96fcc6e3e,
        ],
    ),
    (
        "four_way/false",
        [
            0x317c7359ec315db3,
            0x23b34e5c374558df,
            0x736cba40957b46d1,
            0x729893a6a479d03e,
            0x4e13b2abc5e17a36,
            0x9629afc8dae89704,
            0xc847345fa1054252,
        ],
    ),
    (
        "five_way/false",
        [
            0x4dd45ff0f5a2f5ad,
            0x693a1f2a71770500,
            0x33edca59090a5a8e,
            0xfd6fccfd7595b657,
            0x3fc45ff524fe1fa5,
            0x86cfb034ade20149,
            0xf5ae94634926f7d8,
        ],
    ),
    (
        "six_way/false",
        [
            0x766c12ff380cff02,
            0x4bfbfba0899eb9a3,
            0x4e32e38c3373e27a,
            0xe51465d08f666195,
            0x4c16c6e5467d1837,
            0x453f34b5ea40042a,
            0xbcc22e3363b7bc68,
        ],
    ),
    (
        "y_25/false",
        [
            0xcc8f5838c216f833,
            0x9ecc1ef84c56d123,
            0x315be3be6983a4c4,
            0x1580c222f9f70a21,
            0x5b3eca156fc0b5a1,
            0x3637d84231ed65f8,
            0xedde7593ddfe5af6,
        ],
    ),
    (
        "y_40/false",
        [
            0x2bab644e8dcc4efd,
            0x4db1f47305260d88,
            0x81b60a0c909791a5,
            0xcc7cb32a380f5f14,
            0x6467918fba333ea1,
            0x16eb49e78ad16171,
            0x7fedd4731a300d1f,
        ],
    ),
    (
        "y_60/false",
        [
            0x29e705c080266bc5,
            0x5c0d72ce8c61b6a8,
            0x27c657458c8a83f9,
            0x514e3591273dc545,
            0x5c99ea548082e0b5,
            0xbc0c318e783d371e,
            0xae5f2702b333fb98,
        ],
    ),
    (
        "bend_10/false",
        [
            0x775a078e6709d426,
            0x670c756810bbc426,
            0x61ac512c8a7e3d4d,
            0x04beb00dbe359103,
            0x184a8edb7db27938,
            0x9c80c8207340a2d1,
            0xa9e75f06f0ef2d20,
        ],
    ),
    (
        "bend_30/false",
        [
            0x7ac364ad867f0aa5,
            0xe8b4103e3263b819,
            0x798bf99cbf5e5ee1,
            0xed7ad68229321ed3,
            0x94732f8aa860107d,
            0xd8c492afbcbbee3f,
            0x9caf01df5328dcdb,
        ],
    ),
    (
        "bend_50/false",
        [
            0xdcde0c9c2b570db2,
            0x63d8d9ef2caa3b23,
            0x07d02f0bdc00fdfe,
            0xc5c35206d714ddcf,
            0x006e568bac801fcb,
            0xacd95f68cbfbdf8b,
            0xac3fc7ef308e9802,
        ],
    ),
    (
        "bend_70/false",
        [
            0xe1d6b86bfc4d6910,
            0x372ede5f9936506a,
            0xbe59f6230fd17bfb,
            0x06a8dcea922e7278,
            0xfe6b122261f6f7dd,
            0x3eeb0fd862397592,
            0x8098e52d9bb49c4a,
        ],
    ),
    (
        "bend_90/false",
        [
            0x034fa04904e2294f,
            0xb64f0e2f0d32970e,
            0xb3e6fe959955f2bc,
            0xbc3dc8568fe15ce2,
            0x559bbd2d98daa982,
            0x7e785d041c95bb2a,
            0x084a71e885c0fe89,
        ],
    ),
    (
        "bend_110/false",
        [
            0x1d8fd85cdbb8e965,
            0x6f832a50145a161a,
            0x3536e7899291cf92,
            0x7119f17c68609b5c,
            0x68db2b72e85da3b9,
            0x8efc1838ee6fbf84,
            0x57ee9d220f9287fe,
        ],
    ),
    (
        "bend_130/false",
        [
            0x0d39e17d22569757,
            0xeb80362be445eef9,
            0x52c01e56294613b2,
            0x5dabe68f8c5a0fb8,
            0x5990e4a00a551ed5,
            0xb49b09e9ac8a4496,
            0xa76e0d05e2dcefba,
        ],
    ),
    (
        "bend_150/false",
        [
            0x9d26a68cf5973bc8,
            0xab053dcdc2adf48b,
            0x75c05a9bc3e5d0b5,
            0xfeea57c2c38a59c5,
            0x272ae3fdabd471e9,
            0x5395ded271044d13,
            0x9480dd93d128c3bf,
        ],
    ),
    (
        "bend_160/false",
        [
            0x973e7181d46967ce,
            0xb29b3373ddcf3c3b,
            0x69329698ef6dd6f2,
            0x205f82236c9d85d5,
            0x45d1503c507ce921,
            0xbdfa473324b2ac6a,
            0x07013f89afe6d00e,
        ],
    ),
    (
        "mixed_40/false",
        [
            0x0071ea1ab0551fa2,
            0x42384d36b73969f4,
            0x9fd65550e3364951,
            0x6b32a320b9757096,
            0xeb93baac29b43aa1,
            0x63cea7140438e66a,
            0x69d8865c8ec44a6b,
        ],
    ),
    (
        "mixed_60/false",
        [
            0x09ebe3357e33b039,
            0x336e23d6728d777f,
            0x2fdb1889e1293e27,
            0x531498a57fce9a73,
            0xb88662ef3b06b8fb,
            0x2ed185c30d8e1d04,
            0x5a9f2e53837318e6,
        ],
    ),
    (
        "mixed_90/false",
        [
            0xac6de5d0c31ea2ea,
            0xc4a3e47ed9415aae,
            0x00cf9eea30e3457c,
            0x3e224e92130d7452,
            0x458d3d9d0dab5500,
            0xbc2c1a68b2a165af,
            0x2b41f13ab2dbacd8,
        ],
    ),
    (
        "width_step/false",
        [
            0x5e65109f7a12630f,
            0xc256819376670b70,
            0x9b9c2fb054069d36,
            0x58757d4d37a2c87b,
            0x7ce22f8bb835974e,
            0xae3401bd18689440,
            0xd70461ddcae2aeb7,
        ],
    ),
    (
        "width_four_way/false",
        [
            0x2db528e5346ecc52,
            0xac21bb52e8513d01,
            0x940248c724094194,
            0x1b0083d1c260bf06,
            0x24ea28a1d0e6ff67,
            0x5eb92726f6e2b1c3,
            0x300c510e7947e4a6,
        ],
    ),
    (
        "width_bend/false",
        [
            0xfca7492dafcd8ac8,
            0x1040c5ab78a50009,
            0x5bb7d4152d62b946,
            0x215e2ef8eb8d6f11,
            0xbc7cc385e6469f25,
            0x0776e5d2aa7d1cca,
            0x921ed09aa262400b,
        ],
    ),
    (
        "width_step_convex/false",
        [
            0x4cb626aa593c92c1,
            0x3dc54aa66b0b8dbb,
            0x7fc65774c8d4f1a3,
            0xb369d5b0b47f839c,
            0xb80917c7771428e3,
            0x9b6be5642668c658,
            0xde4aa24e3591a965,
        ],
    ),
    (
        "width_step_miter/false",
        [
            0x540f1688e5bec9c5,
            0x25abd223b58f8afd,
            0xaca4f2d1143036a9,
            0x8826517b97ac0534,
            0x2ac6297434303a24,
            0x8bf409ef60666c74,
            0xd5b4f8352d5124a2,
        ],
    ),
    (
        "straight_convex/false",
        [
            0x4cca132ea60eca57,
            0x66083298660b3344,
            0x2d32202691a3db7e,
            0xcf5207de6c4a9a94,
            0x9290921247cf8ad7,
            0xe21207cc1f8d4d74,
            0xe803d54e09f2cb8b,
        ],
    ),
    (
        "width_step_curved/false",
        [
            0x1c203c3d03fc3540,
            0x2f90758278b6e302,
            0xcc737046812e010e,
            0x15ba23766e5284ea,
            0x039eead7cc45efd6,
            0x65bcd915488b749a,
            0x55420c20ad3efdc4,
        ],
    ),
    (
        "t_corner/false",
        [
            0xc404869e1c98ec42,
            0xd23184bbdca2c88b,
            0x457d5e90e4feca87,
            0x21817faccd645e3c,
            0x9a739388b490cfd4,
            0xaad22d595fe45f06,
            0x2a271a1cb532a620,
        ],
    ),
    (
        "t_beside_arm/false",
        [
            0xfae5731269c658e6,
            0xf3ce22b88a1a9a70,
            0x978f43ffa1524cc2,
            0xf24af5813b42becb,
            0x1ba9df154c32ea9a,
            0xa4279bd7fe13c99b,
            0x563b9ccdbca17b81,
        ],
    ),
    (
        "t_beside_arm_shallow/false",
        [
            0x439d15c880b539a4,
            0x668faaa822d29968,
            0xd10b9ae428b6c557,
            0x38f4c654a7141bc3,
            0x76e2852993964344,
            0x1302435f8e5325dd,
            0xc2ae16d19e7f2034,
        ],
    ),
    (
        "road_end/false",
        [
            0xe0d6ad9633f7a088,
            0x7f0628574840ad31,
            0xd6302dce6b25eba6,
            0xd74c596854100b97,
            0x9f147e1e0be7708c,
            0xb457dae79f0d89b6,
            0x86a67688d1803d79,
        ],
    ),
    (
        "road_end_side/false",
        [
            0x614d84c909c947a4,
            0x33e9769932c54cc7,
            0xaa906dad0536858b,
            0x68dc2417bb19ff14,
            0x30d98ed79d1e6127,
            0x27dc7eeed28ae15d,
            0x6256d4aa96e8f6fd,
        ],
    ),
    (
        "road_end_both_sides/false",
        [
            0xadd6c46921da570b,
            0xbabf918035fdb16d,
            0x73c29ec211b5c6e1,
            0x1522095306adf994,
            0x47a1ae0dd591f3d4,
            0x35bd50df14bb1e35,
            0x6df4ed6295212881,
        ],
    ),
    (
        "through_road/false",
        [
            0x1c9d3857a27f689a,
            0x4002684a90247914,
            0xf9415e9e0e9f7557,
            0x30390d928fdd0e53,
            0x6560b1aeca9d0025,
            0x8bb854ffd8c395df,
            0x758e902f9f18609e,
        ],
    ),
    (
        "bend/false",
        [
            0x57925ac08fb71aa1,
            0x4df1c96083de4ed5,
            0xb2977e92c00a60f3,
            0xe250eaa9fc75a259,
            0xb8c6b6f8ba407bf6,
            0x1c8df6b388e3c34e,
            0x297c2b82edde83ac,
        ],
    ),
    (
        "path_end/false",
        [
            0x53c48b9f4c437b3d,
            0xa8c7e2d6f219ed0d,
            0xecb829966f41560f,
            0xae4f822926d2d50d,
            0x4284d4115c70f2d8,
            0xac2d4e64b6a54f30,
            0xc5fc453cc45788bc,
        ],
    ),
    (
        "path_bend/false",
        [
            0x0c726cad85689f95,
            0x2d5e88953b98ea32,
            0x4c495f41a2aa64e2,
            0xc15f51ef37cebcd8,
            0x481542d5ce100cf1,
            0x98f93616e3d1d72b,
            0xa0320236054a6eac,
        ],
    ),
    (
        "path_t/false",
        [
            0xe5ca42763a18b544,
            0x09290cade347ecaa,
            0x60281d3b0dda91ee,
            0x412ab7c0f7f1ad8b,
            0xb7cad4b6607fa2c8,
            0xacc6e48f3e6a5f8a,
            0xc7591888574ce93b,
        ],
    ),
    (
        "pass_through_north_south/false",
        [
            0x1d20a169849f0b8a,
            0x739ad395cf4530f4,
            0xd992df78674ad366,
            0x999f59cfce2ba162,
            0xfe12bcf52a72cfd7,
            0x988ec4648997434f,
            0xbe5887f6f769723e,
        ],
    ),
    (
        "pass_through_south_north/false",
        [
            0xbd14f42475d6ef56,
            0x392928826e5e6b74,
            0x1a636d3b1c6c139e,
            0xd7bd38fb72d2ab93,
            0x704e9fad41018f87,
            0xb9cee177f11ecc5e,
            0x1e1147acb598547b,
        ],
    ),
    (
        "pass_through_bent/false",
        [
            0x07f88a750c26363b,
            0xb77c183789575753,
            0x3e1de72bc5e9f637,
            0xa8180aaf92477ff2,
            0x937af35d11d250a7,
            0x80bc0c192c1e37d3,
            0x9029c4157495db69,
        ],
    ),
    (
        "t_junction/false",
        [
            0x37b914ab9643a5f6,
            0xc25ade11116a8f8e,
            0xa4096f090e1eb816,
            0xbcc264cf67760a9f,
            0x5fa1a941d25c43a2,
            0xe366140a5a930ca8,
            0x5343aae81d65bc9d,
        ],
    ),
    (
        "mixed_profiles/false",
        [
            0xec4c07ba36422c4a,
            0xa6369ba97d077c08,
            0xc383104cf7c463fa,
            0x7d95edf6377fdc93,
            0x58bcf2dd036e912f,
            0xf3873433a72dbc69,
            0x94cfa8314927654f,
        ],
    ),
    (
        "bend_15_120/false",
        [
            0x6d3c2abe6068e9a2,
            0xe1a200a5b7506271,
            0x6b8d4c12013872d5,
            0x8ffb3cbdb436ccd4,
            0xc332377a901bb34b,
            0xa009ba39e20f4c17,
            0x07f24f32a65b3fe9,
        ],
    ),
    (
        "rural_t/false",
        [
            0xea1946252ff46746,
            0xac35bb26229c151e,
            0xc8ea79d6125150b9,
            0xd6930a45f3eb724f,
            0x8228805f03791825,
            0xce6d5b38cb278b06,
            0x4daf5579e5c097a4,
        ],
    ),
    (
        "rural_four_way/false",
        [
            0xfa6d31a13d93bd53,
            0x010e211b60960aa2,
            0x2bbf8f04492bf53f,
            0x5ea845ede2ab4450,
            0x5cb86659e9cd3c40,
            0xcdfb1bdd333fbf18,
            0x1d8bc91ebbe52d22,
        ],
    ),
    (
        "rural_y_40/false",
        [
            0x81e91ccb954cf777,
            0x965ab172b5484c98,
            0x1fda42f488af70ca,
            0x22bff98fff888a1e,
            0x8228805f03791825,
            0xf4f02f77731b66de,
            0xb099c7673d1b4bf7,
        ],
    ),
    (
        "rural_bend_90/false",
        [
            0x447479e64d7331a0,
            0x0fe6d5aecebb69b0,
            0xfff1f58eff724ed0,
            0x8f99b666decb48ad,
            0x3bdbba5dd0d3e707,
            0x81f625793e33cc02,
            0x3cc50f8c3b309d04,
        ],
    ),
    (
        "rural_bend_150/false",
        [
            0x6eef1a19d03591dc,
            0x5c19e507deb16f95,
            0x596e3e8266946b1f,
            0x49956c0d2149c40e,
            0x3bdbba5dd0d3e707,
            0x34e6d222c0f675c7,
            0xe8f3775019d89062,
        ],
    ),
    (
        "rural_side_road/false",
        [
            0x5adec4ac0a31114a,
            0x0529e946e070e205,
            0x437c0caff2ebc11b,
            0xbe37168d203d93e1,
            0x8eba1e9904e87a9a,
            0x21145da6227d8bd2,
            0x3547ced7f6434d73,
        ],
    ),
    (
        "rural_through_road/false",
        [
            0xf3202e1d2224e260,
            0x3de4045b89fe774d,
            0xf66ba7008dd7180d,
            0x7d941c92d6afa4c7,
            0xbf7d38ca28f0f5dc,
            0xbd02eede30d7c984,
            0xc0a52637dd77f4e9,
        ],
    ),
    (
        "rural_wide_t/false",
        [
            0xdb3a9670e55b4f03,
            0xa1dbaf1768ba17b3,
            0xbf6dbc6f9eb22fdd,
            0xb9cccffffca99de8,
            0xe055a2690e4a7036,
            0xf6f19cb9a618f57e,
            0xde84211bc90b91c1,
        ],
    ),
    (
        "rural_bend_mixed/false",
        [
            0xa3199bfc8e88e10a,
            0xb105f24403a1b224,
            0x167d32e1ee42ddd2,
            0xc9f422cc3c505f00,
            0x013f55c3e6d5e930,
            0xdf5809c596684b01,
            0x1917c676ecfcac71,
        ],
    ),
    (
        "band_step/false",
        [
            0x05fb62bd201ea30b,
            0x12888b835a326539,
            0xa68c0e6478c0b37f,
            0x97b23eda97736608,
            0x38163080eacffb20,
            0x1132ac7e36700219,
            0xddec630eb0cc71fe,
        ],
    ),
    (
        "band_width_step/false",
        [
            0xc9cc4922efc6d45b,
            0x2264a89608dcc4c4,
            0x9f8e4c2b14212330,
            0x20dc72e3886f1c57,
            0xc5118c8ec2865d26,
            0xd86ce8a5c1400a9c,
            0xfe9ba89676272534,
        ],
    ),
    (
        "band_four_way/false",
        [
            0x5457f7a9bdfc8d49,
            0x766ddb2abf1b0ba9,
            0xc08792f5d633a33a,
            0x64c114a56db8de8b,
            0x9e2d6ec2592a9313,
            0x455919e8864e3de7,
            0x476c38ae3dfceef3,
        ],
    ),
    (
        "band_pass_through/false",
        [
            0x5b1410d9ee659514,
            0xfef4edae3651fd6a,
            0xd57f75b31a066bf4,
            0x0eda8952cf9e7389,
            0x54a2f0df16362e8d,
            0xcdfa215d1d4039b6,
            0xa947f86bef7fc175,
        ],
    ),
    (
        "width_pass_through/false",
        [
            0x1028ad1183c67bb1,
            0x1152944f80dda4d3,
            0xe099e4c1df05e15a,
            0x60b9bb8e167f2582,
            0x09bf38fbe27bee6f,
            0xf4fc9b244fdf25c2,
            0x29af9b06450669ea,
        ],
    ),
    (
        "width_bridge_pass_through/false",
        [
            0x74d1d856a26040e7,
            0x6b50d483186356d3,
            0x605fc6fbf3e05500,
            0xd647dd18e9724f73,
            0xcaef0c70398172e7,
            0xb17ff9f89b5524ce,
            0x00624c2e000e6791,
        ],
    ),
    (
        "bridge_t/false",
        [
            0xa603b47fc69a2ebf,
            0x316cd30bc4c6d451,
            0x6916fbee27fd7d64,
            0x8b56e55466aac1ca,
            0x38af174d571d9151,
            0x571d441d536b25b3,
            0x119add5f57c00b2f,
        ],
    ),
    (
        "bridge_bend/false",
        [
            0x494b531707c6e34d,
            0x6ee62ae73d765385,
            0xc3bd732dc7c3d626,
            0x902b842a926ec651,
            0x59ad34c2b668beaa,
            0xe408d834a54e7742,
            0xa6d5df1f655f20ff,
        ],
    ),
    (
        "bridge_pass_through/false",
        [
            0xee10df481aa5732d,
            0x07579bad55785ce6,
            0xa56e334196ab5f26,
            0x5f16ab208983483c,
            0x7fcdf0ff7796cf61,
            0xc98400e7f37d360e,
            0xe5a7c5d39d2f2453,
        ],
    ),
    (
        "bridge_rural_t/false",
        [
            0x189dc1c0fd682e7b,
            0x8e257c68d2b3787a,
            0x88696d0184cea9e9,
            0xf33dbf05c9c16af4,
            0x8f65bfbe62aadcc4,
            0x1e763e57358878d1,
            0x9ba901a1454c6ef8,
        ],
    ),
    (
        "tunnel_t/false",
        [
            0xc7e956e72dd72ebf,
            0x316cd30bc4c6d451,
            0x6916fbee27fd7d64,
            0xc9e276371e4d3c45,
            0x38af174d571d9151,
            0xe4c6e9a1f5b42fb3,
            0x8e7bf2e23f88088c,
        ],
    ),
    (
        "tunnel_bend/false",
        [
            0xe9ee337eaf5047cd,
            0xda08c0c406e26bbf,
            0xc3bd732dc7c3d626,
            0x3c09f9cedfc53333,
            0x47b9e4ef0180be6c,
            0xebd97cc85440c1c2,
            0xc02834496ead130e,
        ],
    ),
    (
        "tunnel_pass_through/false",
        [
            0x4f05dc7283f3782d,
            0x07579bad55785ce6,
            0xa56e334196ab5f26,
            0x721012b90ddd8bd9,
            0xb453e5bd6aa15521,
            0xb15926a2affcc66c,
            0x808d87d84942f9b0,
        ],
    ),
    (
        "band_step_curved/false",
        [
            0x3611329db269bf95,
            0x9a402820e8e8a349,
            0x553418b6df056dd5,
            0xa44e76d2e5783725,
            0xb6bfa69ee37e1664,
            0x831dcc8cb3dea32c,
            0x39011dedef0aabe8,
        ],
    ),
    (
        "band_width_step_curved/false",
        [
            0xf9ef8cb16546eef3,
            0x2f90758278b6e302,
            0xcc737046812e010e,
            0xd63d9a1710198b53,
            0x039eead7cc45efd6,
            0x3bbfd82bc5cd6c52,
            0x7b4e6542dc9a87ef,
        ],
    ),
    (
        "t/true",
        [
            0x09bf787a8558dea6,
            0xdcf70105d785ee63,
            0xacdced0eba6074f4,
            0x86c2405972c2f524,
            0x07be64c902173d1d,
            0x4e226e2c2c3da8d5,
            0xa6f887a6627b0030,
        ],
    ),
    (
        "four_way/true",
        [
            0x2475b6d16c1cd349,
            0xeceddda179040bc0,
            0x736cba40957b46d1,
            0x17dfbf3e1887c9e6,
            0x6a8367b44fa47217,
            0x674f7dec545f9d3d,
            0xb1b7311d180f87f6,
        ],
    ),
    (
        "five_way/true",
        [
            0x40af52ccfd3ddab8,
            0xd79656422b429cb1,
            0x33edca59090a5a8e,
            0x30f64c8b61e78e7a,
            0x5957204fd60cdcdc,
            0x17a9472507668c0a,
            0x0522608424ba1673,
        ],
    ),
    (
        "six_way/true",
        [
            0xdd4062bce667268e,
            0x493ce05cf430c4aa,
            0x4e32e38c3373e27a,
            0x03c1de5cb60c1f7a,
            0xe928217af7569330,
            0x7b7b0c979a16ccec,
            0xbba96cd883ff4ba2,
        ],
    ),
    (
        "y_25/true",
        [
            0x25a6eb9dd02d04d1,
            0x84e15068f15d8c1f,
            0x315be3be6983a4c4,
            0x077326ce5be8928e,
            0x31a90561da346ac2,
            0xcc5316bc6d11a522,
            0xc242e9627ddf9bd0,
        ],
    ),
    (
        "y_40/true",
        [
            0x29a0779b37f6a64b,
            0xe19783055f657987,
            0x81b60a0c909791a5,
            0xb7965dcfb10e705f,
            0x7d59a6109534cbbb,
            0xf99e0e79459d2382,
            0x522cd03e06905c8c,
        ],
    ),
    (
        "y_60/true",
        [
            0xa290c3f599db3ff1,
            0x7b697c82f3d42fb0,
            0x27c657458c8a83f9,
            0x18191a3801ddac1b,
            0x37d6b21874ca12bf,
            0x09f689da33062e64,
            0xc79eb745ae6ae0e2,
        ],
    ),
    (
        "bend_10/true",
        [
            0xb50fb88e95c7cfb7,
            0xe7c0ffd44139383d,
            0x61ac512c8a7e3d4d,
            0x399fa33dd91059df,
            0xb4416ea6abad4253,
            0xf84afd12c8d83330,
            0x171173a1f3380b82,
        ],
    ),
    (
        "bend_30/true",
        [
            0x7bc5d1f139716aca,
            0x69dcb2487acb491b,
            0x798bf99cbf5e5ee1,
            0x921d7d61107aedf6,
            0x4b6ef671422847ac,
            0x603ec297ea17096a,
            0x73ff6ce8f0199b36,
        ],
    ),
    (
        "bend_50/true",
        [
            0x887c1798d6e6b6cd,
            0x6c7aaec7e2b56d7f,
            0x07d02f0bdc00fdfe,
            0x05114b50b787373b,
            0xdbb330e0d1a6ebc2,
            0xa3f0d7e63ec739dc,
            0x6ef6503ed5a9e042,
        ],
    ),
    (
        "bend_70/true",
        [
            0x4dac26038649f7b9,
            0x0046d498c0339c91,
            0xbe59f6230fd17bfb,
            0x404b0275d259f6f9,
            0x9956879a56f16107,
            0x6db537df861cfed4,
            0xeabd282de5bcef3a,
        ],
    ),
    (
        "bend_90/true",
        [
            0xb118e0bc17c65d8d,
            0x5ae1e97088035a0f,
            0xb3e6fe959955f2bc,
            0xb4019e8a5d499960,
            0xefdf37fea13a0b8a,
            0xcc55de23a9ffe578,
            0xf782fb8515681ea6,
        ],
    ),
    (
        "bend_110/true",
        [
            0x43a919731fb3e77a,
            0x28efa15c3f86883b,
            0x3536e7899291cf92,
            0x21941731788e2029,
            0xc4e782ba6cdf9209,
            0x656608d1dfc3d582,
            0xd2bdc0c0365f30bb,
        ],
    ),
    (
        "bend_130/true",
        [
            0x56ee47bcdd945b8c,
            0x99debeb25ca8fa04,
            0x52c01e56294613b2,
            0x4ad98a0a2cdd729c,
            0x9aa7192f3419e71f,
            0x78491c4ca496c930,
            0x7d8b8c52e7942338,
        ],
    ),
    (
        "bend_150/true",
        [
            0xf94ff09f5b57fd37,
            0x68b51e58d2873a76,
            0x75c05a9bc3e5d0b5,
            0x37d85b213db3ac5b,
            0x0daa25d66b48c5f0,
            0xb46b25119c44c3f4,
            0x553c5ee1cb851e1d,
        ],
    ),
    (
        "bend_160/true",
        [
            0xe5bddfc61daf20b9,
            0xbb62cce6548b512c,
            0x69329698ef6dd6f2,
            0xdcbc5b68f8174d6a,
            0xf68f4cd7462a00ed,
            0x833ff2f7753cbd53,
            0xb89d564a500e552c,
        ],
    ),
    (
        "mixed_40/true",
        [
            0xd40d25cfffc7653d,
            0x17b60781d46ebbeb,
            0x9fd65550e3364951,
            0xcbcf1382978e1089,
            0x98e2e03c4a358086,
            0x44b0382546b0ea7e,
            0x3bbf020bc9a60675,
        ],
    ),
    (
        "mixed_60/true",
        [
            0x78eb76985a30b0ec,
            0x323f6b8a61513dbc,
            0x2fdb1889e1293e27,
            0xed84a1f411c4998f,
            0x2b3e214948c2a274,
            0x79ad2a9b1c704aac,
            0x8e6ed91648dfbb5b,
        ],
    ),
    (
        "mixed_90/true",
        [
            0xc78867cc715b12ed,
            0xbad21aafe86dc9b9,
            0x00cf9eea30e3457c,
            0x55d468fd676df0cf,
            0xda7a7f554ddd1626,
            0xc9feaf496595f126,
            0xdd2ee11045005e20,
        ],
    ),
    (
        "width_step/true",
        [
            0x4ce06d794ecf767e,
            0x5cb09c1bc212879a,
            0x9b9c2fb054069d36,
            0x97f3ac0aeed745da,
            0x705a494355c5e36c,
            0xfd41b6b933777a29,
            0x53dd411be5af6660,
        ],
    ),
    (
        "width_four_way/true",
        [
            0x6088efadaca82ad0,
            0xfd05f04deb65979e,
            0x940248c724094194,
            0xa99543c7b357c007,
            0x7f3c3c9940cf8639,
            0xa6d920a077eeadd2,
            0x68e9b156097c48ca,
        ],
    ),
    (
        "width_bend/true",
        [
            0x1ee16d9489e48786,
            0xd49d8afc16829f68,
            0x5bb7d4152d62b946,
            0x0a395116b26c133e,
            0xb2ceec6ce4fc2b8f,
            0xf2e52401077083d4,
            0x5c05ecc6f996140c,
        ],
    ),
    (
        "width_step_convex/true",
        [
            0x5163579d646f9a50,
            0x6d652a61b9579263,
            0x7fc65774c8d4f1a3,
            0x87584e2c4addde5a,
            0xfa2afc684132942d,
            0xa8047ffa1610a3f8,
            0x6e750118df5a3bf5,
        ],
    ),
    (
        "width_step_miter/true",
        [
            0xae6196011608fd54,
            0x8e6a8557836188cc,
            0xaca4f2d1143036a9,
            0x6bb47d1d98585f26,
            0xf632ec9acae4d497,
            0xd8ed5854e7e4002d,
            0x629ab984f3b8d278,
        ],
    ),
    (
        "straight_convex/true",
        [
            0x2e74342f6e972a16,
            0x3084deb2a6a405ab,
            0x2d32202691a3db7e,
            0x57f1db42d4fb82d5,
            0x5357ef50211f078d,
            0xca3726161e32713c,
            0xf08be62d60113ccc,
        ],
    ),
    (
        "width_step_curved/true",
        [
            0x0fc6313064d263a3,
            0x3d478e05a0b21a72,
            0xcc737046812e010e,
            0xbd0c8c7902a3fec1,
            0x4173e3aec4cc3d37,
            0xf86371e1c2ad91a6,
            0x6163726b0709187c,
        ],
    ),
    (
        "t_corner/true",
        [
            0xdf435b050781d8a3,
            0xc5f42d8a063a49e0,
            0x457d5e90e4feca87,
            0x52f7d62545c9d6f1,
            0x4818c07867941157,
            0x547e1e32b0ad150e,
            0x94b123b17d78d049,
        ],
    ),
    (
        "t_beside_arm/true",
        [
            0xb242861497d045bc,
            0x87317297179a4256,
            0x978f43ffa1524cc2,
            0x10bf6a604c9b1239,
            0x67cbf1a34f48dea4,
            0x7440b17638e6a1f1,
            0x2309f7ff39ec56ea,
        ],
    ),
    (
        "t_beside_arm_shallow/true",
        [
            0x7f58835cefbaeecf,
            0xa6796c461ccf1811,
            0xd10b9ae428b6c557,
            0xee332f4651c93f98,
            0x897a0ab845afe88a,
            0x5580a46a91225bbb,
            0xe51fb3aefe372d7c,
        ],
    ),
    (
        "road_end/true",
        [
            0xe4804cc10253bb59,
            0xd19511c21f623ebc,
            0xd6302dce6b25eba6,
            0xe16228b56851483a,
            0x0c3d618f8890275b,
            0x8b64f4a039abfd3b,
            0x35178ca493dc131c,
        ],
    ),
    (
        "road_end_side/true",
        [
            0x66b1c60ae8229236,
            0xb11eaf5dc524d939,
            0xaa906dad0536858b,
            0xfa18abb591cca8e5,
            0x2062e8e94fd58c52,
            0x6736ef263d8edc5b,
            0x22ea9ec9f8d3376b,
        ],
    ),
    (
        "road_end_both_sides/true",
        [
            0x920f028705ae9818,
            0x22e3b7b98abe32c6,
            0x73c29ec211b5c6e1,
            0xe9cff55fd60331ed,
            0x4ef4366a314bcac0,
            0x98051c52d56d065f,
            0x276d075aff2bc61c,
        ],
    ),
    (
        "through_road/true",
        [
            0xcf83a2d7021eedb9,
            0x8b878fd7c9962359,
            0xf9415e9e0e9f7557,
            0xb4d78f8af20e866c,
            0x03534e22dc8c1b3d,
            0x0b3ea39c020bdbcc,
            0xe983515cb30c03aa,
        ],
    ),
    (
        "bend/true",
        [
            0xcefcfca48903cc94,
            0x16da794da439a0d4,
            0xb2977e92c00a60f3,
            0x90c2509af97fdb45,
            0xa216cd2bd942cde3,
            0x8d9887db47453c35,
            0xbbb62b29c2abf0a3,
        ],
    ),
    (
        "path_end/true",
        [
            0x728cdf1e7380c780,
            0x09b450d8dd2aa05b,
            0xecb829966f41560f,
            0x0521d30890d369e2,
            0xfe28c50d6746abfc,
            0x3722c48309bcbc30,
            0x6fe83317893115d3,
        ],
    ),
    (
        "path_bend/true",
        [
            0x93825ca8e4738c7d,
            0x69b5978f06f7ac64,
            0x4c495f41a2aa64e2,
            0xe62017ac4633285f,
            0xfd240a591fa0ad7e,
            0x125b24e66b8ef083,
            0xcedb5b07385a70a3,
        ],
    ),
    (
        "path_t/true",
        [
            0xae24fcb7e6fe189d,
            0xc4c44e116a3bfff7,
            0x60281d3b0dda91ee,
            0x86116748b96252ec,
            0x000947d02d538f8e,
            0x08c52150051075df,
            0xd7d0574801a159a9,
        ],
    ),
    (
        "pass_through_north_south/true",
        [
            0x52974db4c77bff0b,
            0xfd6b1caefc91fc6d,
            0xd992df78674ad366,
            0x56527c69f7bed5b4,
            0x022b043cb431fbf0,
            0xa984bce2ec8f4935,
            0xaf92431da0127304,
        ],
    ),
    (
        "pass_through_south_north/true",
        [
            0xbfac4f2adbfc61d7,
            0x8b83efd9d9fea815,
            0x1a636d3b1c6c139e,
            0xeb84ebfad15d940e,
            0xecc33b6a90b7cf38,
            0x9480d77a6d43cac8,
            0xa2e735efc5c1b5c2,
        ],
    ),
    (
        "pass_through_bent/true",
        [
            0xbfffdfc4a74283ba,
            0xe2e32459d62583ec,
            0x3e1de72bc5e9f637,
            0xe818c4e3c6fa3f88,
            0x2d7548610a8388dc,
            0xa13aef1816c9fa75,
            0xd3ead915bb5ed701,
        ],
    ),
    (
        "t_junction/true",
        [
            0x1ea44dbbb9126b52,
            0x89ea6e026af3830a,
            0xa4096f090e1eb816,
            0xfd5748ffff040e0d,
            0xe93e6e16036aad50,
            0x44ffe31e940bd777,
            0x6c0096cd4b7a3c52,
        ],
    ),
    (
        "mixed_profiles/true",
        [
            0x5e54ec7e837e70d8,
            0x1883b8fc6dabb3e9,
            0xc383104cf7c463fa,
            0x7e2147920c30e383,
            0xd14f36a5dc84b374,
            0x034b9da1220567a1,
            0xa932f83299d975ed,
        ],
    ),
    (
        "bend_15_120/true",
        [
            0x035bcdd6dbf8e39c,
            0x987b10e41de1498e,
            0x6b8d4c12013872d5,
            0xbf6b93917fe66880,
            0xc992cacdbbb6d723,
            0xe9954e48d3be1773,
            0xcd4bf4b32e91ff8f,
        ],
    ),
    (
        "rural_t/true",
        [
            0xe520d810ecc1d54f,
            0xe2b4ae5440848445,
            0xc8ea79d6125150b9,
            0xdfc736d0bee4f0d9,
            0x8228805f03791825,
            0xebc89d39af5514cd,
            0xbb5edfe03bfa4097,
        ],
    ),
    (
        "rural_four_way/true",
        [
            0x9c4bc5c2d5b3d5a3,
            0x467373eae77ca49a,
            0x2bbf8f04492bf53f,
            0x70528287522abda1,
            0x5cb86659e9cd3c40,
            0xc125b78c78611afc,
            0x2910301dd65cecc1,
        ],
    ),
    (
        "rural_y_40/true",
        [
            0xbf568e94ac09aa0f,
            0xcdef6acbcce93201,
            0x1fda42f488af70ca,
            0x0c42f93e6fe08bcd,
            0x8228805f03791825,
            0x1d1057c1b8a5bb5d,
            0xfd62cafb41d0a45d,
        ],
    ),
    (
        "rural_bend_90/true",
        [
            0x96cbd152adda7048,
            0xda95e3cf0d53f222,
            0xfff1f58eff724ed0,
            0x543fe9deacf96655,
            0x3bdbba5dd0d3e707,
            0xd7426c92e71e3066,
            0x007aa72cd7322f7c,
        ],
    ),
    (
        "rural_bend_150/true",
        [
            0x1603ee2e8ffce984,
            0xd115055effcaa8d4,
            0x596e3e8266946b1f,
            0x94d73d0844379eb0,
            0x3bdbba5dd0d3e707,
            0xa5259d81756c55fa,
            0x8b6a0f4384f0c946,
        ],
    ),
    (
        "rural_side_road/true",
        [
            0x86498cd789f973f3,
            0x9950595d4e93d81d,
            0x437c0caff2ebc11b,
            0x1c4cde8b33c89a35,
            0x5beb5a3a0e88a0e2,
            0x38278d4dd886201d,
            0xcc8c16463c2afa33,
        ],
    ),
    (
        "rural_through_road/true",
        [
            0xcd011ad4858f4b58,
            0x8cac4d09b4bfeac3,
            0xf66ba7008dd7180d,
            0x9365cb065ee8d446,
            0x9e33f142af92f064,
            0x3815f4ff08948322,
            0xb0c8ef84e1bad20d,
        ],
    ),
    (
        "rural_wide_t/true",
        [
            0xae0e10470a66f026,
            0xeef1dbf6d9670445,
            0xbf6dbc6f9eb22fdd,
            0x75645ba71aa2b463,
            0x3b16cd939b604c1e,
            0xedeb512d4f8b7e3e,
            0xa2e3f6c2161967dd,
        ],
    ),
    (
        "rural_bend_mixed/true",
        [
            0xaa5b5a46d45b7432,
            0x8012e6c35bb51e66,
            0x167d32e1ee42ddd2,
            0xd79b007e8ecd789e,
            0x0f17efcf9b4ec774,
            0xd0589a3f0c3d11e4,
            0x0a4098329d38a3ad,
        ],
    ),
    (
        "band_step/true",
        [
            0x712d272262989db1,
            0x681aac560d07dfda,
            0xa68c0e6478c0b37f,
            0x040793ed8239f4a4,
            0xdb1f9953b16c7363,
            0xeda876d7d84476dd,
            0x68ff34c52f8e242b,
        ],
    ),
    (
        "band_width_step/true",
        [
            0x6dd939246e425097,
            0xf93c61b70a683c14,
            0x9f8e4c2b14212330,
            0xc69d37c7ef87647d,
            0x25ed1a44f334cf71,
            0xd7e17eab5bb51b26,
            0x9eb78b2fcb98f8ec,
        ],
    ),
    (
        "band_four_way/true",
        [
            0x877a47385d4cbf4a,
            0x9bc0bb6d2460453b,
            0xc08792f5d633a33a,
            0x5f3e76f2ce21917d,
            0x21e5e1ae4150c7c3,
            0x70baa22bcbd3e673,
            0x8771c3bf907c3bf0,
        ],
    ),
    (
        "band_pass_through/true",
        [
            0xde7168f7faea0282,
            0x6818d860fc401ad5,
            0xd57f75b31a066bf4,
            0xc05584965bd77516,
            0x589e8bbdf0a5421a,
            0x1992e822e5efed04,
            0x2de9be3c51fbc320,
        ],
    ),
    (
        "width_pass_through/true",
        [
            0xb9fab945f4b07411,
            0xc4e101114ae8f0f4,
            0xe099e4c1df05e15a,
            0x7223b1026a97e761,
            0xdccb1b1b8710d471,
            0xdf0c24ae61b0d13a,
            0x85bec03cb10de5ff,
        ],
    ),
    (
        "width_bridge_pass_through/true",
        [
            0x8eb98bec10b58f08,
            0x31eac905302a9a61,
            0x605fc6fbf3e05500,
            0x6d613c895ea5efef,
            0x2b904b49bc9f4a15,
            0xc4463066a0e80bb1,
            0xd29e4c9f43c0ada6,
        ],
    ),
    (
        "bridge_t/true",
        [
            0x924f7ebf749ebb06,
            0x1ddcb9d3a4cff34d,
            0x6916fbee27fd7d64,
            0x080dc420db0aa701,
            0x809c7d55d4aaeb50,
            0xc72114ad9ee15f79,
            0xb2407e7cb574adc1,
        ],
    ),
    (
        "bridge_bend/true",
        [
            0xf33eecb20cb54e1e,
            0xa88277e7361761f1,
            0xc3bd732dc7c3d626,
            0xfef9b9b37b43c418,
            0xdb11efc5876ac323,
            0x241b0af94b158fee,
            0x456e65c7b1035198,
        ],
    ),
    (
        "bridge_pass_through/true",
        [
            0x097ac38a1d10c8c9,
            0x01a2923d59ff37cc,
            0xa56e334196ab5f26,
            0x2938b61ec8e471a5,
            0xc8cee1cb247ff6c0,
            0x925995558b39682d,
            0x33f22233aee1c429,
        ],
    ),
    (
        "bridge_rural_t/true",
        [
            0xb856bf1122f3e32a,
            0x757f7c9a909415ad,
            0x88696d0184cea9e9,
            0xd8720ecb32d7eff5,
            0xf240e7ca7b5c7169,
            0x320a232c7ee2827b,
            0x976922a1d3f99cff,
        ],
    ),
    (
        "tunnel_t/true",
        [
            0x522ec5c67b80d40e,
            0x1ddcb9d3a4cff34d,
            0x6916fbee27fd7d64,
            0xf1e17f13bf599ba0,
            0x809c7d55d4aaeb50,
            0xf895f7d6581a18f9,
            0x0bfe5f41c78bc114,
        ],
    ),
    (
        "tunnel_bend/true",
        [
            0x5bb9074887d9d00a,
            0x605b0e92a436dba2,
            0xc3bd732dc7c3d626,
            0xc9e865bce96f1b22,
            0x783a19c1f4b3a36f,
            0x3c7402c7b3754baa,
            0x0dc1804e1c3978cb,
        ],
    ),
    (
        "tunnel_pass_through/true",
        [
            0x8315a3446fa6d340,
            0x01a2923d59ff37cc,
            0xa56e334196ab5f26,
            0x5eb1a708b0a700f6,
            0x6cce29be6e4ef515,
            0x05e0313b74925dba,
            0x742acff099dc78fa,
        ],
    ),
    (
        "band_step_curved/true",
        [
            0xdc7afc910bfcb601,
            0x2335c22af714f289,
            0x553418b6df056dd5,
            0x8008a9cec9552ac4,
            0xb8a9ed7b9cde8b8b,
            0x271f31d32fbcedc7,
            0x2d78a4949e3dcddc,
        ],
    ),
    (
        "band_width_step_curved/true",
        [
            0x2fa6d4b379df1832,
            0x98942f7e690352e2,
            0xcc737046812e010e,
            0x50289483ec5f3e0f,
            0x219584b67a3a5be2,
            0x18906559153265f8,
            0x1c28a55dc8c969f4,
        ],
    ),
    (
        "short_link_10",
        [
            0xb0a04063a031538d,
            0xcb941afa435c71e2,
            0x2356a11e5180a8cd,
            0xefcaec251cce2c0a,
            0x3cb6e3049aca947f,
            0x6559bfc7910d2c29,
            0x577360f0120a5b49,
        ],
    ),
    (
        "short_link_13",
        [
            0x4e1b7629ca071646,
            0x86cb2d1f9841defd,
            0x7f60bb6593bd8f06,
            0xf7232f042835da93,
            0x1ce16bfca582ec5e,
            0x7127c8f0b462981c,
            0x23ffeb35e9b29ca9,
        ],
    ),
    (
        "short_link_16",
        [
            0xb2a628dc458d1774,
            0x3c890e1b2020bcd4,
            0xd1cfa355e519c9a6,
            0x9d18e685216e5342,
            0x271153a70eb027ff,
            0x175c6ad077d0931e,
            0x270cb8bba3fd37ff,
        ],
    ),
    (
        "short_link_20",
        [
            0x4d34d4585d943ce6,
            0x5934cfed06d56509,
            0xbebd8bbfd6b9b429,
            0x114cb676fdae13d1,
            0x5546ec0e4ef917ad,
            0x14e819d2a7481e2a,
            0xc0961f6ccef62c9f,
        ],
    ),
    (
        "short_link_25",
        [
            0xa32a2a191cabb9f7,
            0xcc0df1b782e65bab,
            0xba5fecfd56d53b38,
            0xcc5f517278387933,
            0x76d37a58c1b9af42,
            0x828eb272dd0e083e,
            0xf2beee532da9c73c,
        ],
    ),
    (
        "short_link_30",
        [
            0x4c350e76df9f9777,
            0x6c816c2f406f7725,
            0xe791180393f20118,
            0xdd9d2692e35096d3,
            0x33ad445c74bdf167,
            0x3a6671ecd7ce54d1,
            0x7bff01203ff6cd75,
        ],
    ),
    (
        "short_link_40",
        [
            0x06388a274450ea9d,
            0x0df471f300801a41,
            0xa047deeb550659fd,
            0x2877afe0b7bc0496,
            0xbdebdcb1d3aef51a,
            0x8822828b18558d59,
            0x87926626f99140ad,
        ],
    ),
];

// Recorded by `TERRAIN-07-B`: the strip layer is new, so every entry is new; the seven
// top-surface hashes above are unchanged. Re-recorded by `TERRAIN-07-C` round 2: the 18 entries
// with a structure whose far end is free (the bridge and tunnel families and `mixed_profiles`)
// change with its end cap; every other entry and the top-surface hashes are unchanged.
const EXPECTED_STRIPS: &[(&str, u64)] = &[
    ("t/false", 0x3a3196117e3f00f3),
    ("four_way/false", 0x694f53237d5dd070),
    ("five_way/false", 0xbd855df09fc104c9),
    ("six_way/false", 0xd06d91b52b42d2ea),
    ("y_25/false", 0x77d498d4b7c23609),
    ("y_40/false", 0xcc1ad8c55c4b1d83),
    ("y_60/false", 0x5d157a95ce429b3d),
    ("bend_10/false", 0x454964b9141f7eb0),
    ("bend_30/false", 0x1347185fe1f2d040),
    ("bend_50/false", 0x834481f7fcdcd89e),
    ("bend_70/false", 0x8627febafcef7e00),
    ("bend_90/false", 0x93a2ae4d987f7846),
    ("bend_110/false", 0xb686f120f886c200),
    ("bend_130/false", 0xaa216dd22f845740),
    ("bend_150/false", 0xf98f455a8a8d34fc),
    ("bend_160/false", 0x924a6d01e80a5d8e),
    ("mixed_40/false", 0xf30d9201a629d139),
    ("mixed_60/false", 0x2c6d1fb14386c87d),
    ("mixed_90/false", 0x2c44dad5db28eab7),
    ("width_step/false", 0x0e0b8be43a70fca3),
    ("width_four_way/false", 0x28c86ce98b0b1560),
    ("width_bend/false", 0xdafd027cf349042e),
    ("width_step_convex/false", 0xbe6a5c8cbff6bed9),
    ("width_step_miter/false", 0x86ffbbc27fafb1a9),
    ("straight_convex/false", 0x2cdcaa313b96628d),
    ("width_step_curved/false", 0x4f2fc9706d3d0149),
    ("t_corner/false", 0x58fcc8c3893ddbb8),
    ("t_beside_arm/false", 0x981f6e58f57049b6),
    ("t_beside_arm_shallow/false", 0x1693c23696144d0c),
    ("road_end/false", 0x1e896c2b8f6e1180),
    ("road_end_side/false", 0xe93e993aa12f9e6c),
    ("road_end_both_sides/false", 0x44b4fc0de7e70cef),
    ("through_road/false", 0x3e743e6a6e706f69),
    ("bend/false", 0x9a1c3a1198d4b48a),
    ("path_end/false", 0x534fbfb9ae7afa81),
    ("path_bend/false", 0xc3707d0d4e545f7a),
    ("path_t/false", 0x1bd24bb8b0a33fdb),
    ("pass_through_north_south/false", 0xf250f1e394d6c257),
    ("pass_through_south_north/false", 0x6bf47f9b50153147),
    ("pass_through_bent/false", 0x7eaa93e7bed6f463),
    ("t_junction/false", 0x1deed8b4b6144fcf),
    ("mixed_profiles/false", 0x2030c8d4490b81bd),
    ("bend_15_120/false", 0xa94c7a07fb616e46),
    ("rural_t/false", 0x1a30591260f5fc65),
    ("rural_four_way/false", 0xe682cf22d5dcaeac),
    ("rural_y_40/false", 0xdf75f8101826b055),
    ("rural_bend_90/false", 0x7f52c835628dee88),
    ("rural_bend_150/false", 0xf771fab92e82554c),
    ("rural_side_road/false", 0x38adec74719be30d),
    ("rural_through_road/false", 0x3ce5326ab8561a53),
    ("rural_wide_t/false", 0xf95cc94cf03a6827),
    ("rural_bend_mixed/false", 0x6eb822be6bbf2200),
    ("band_step/false", 0x0a43b5d21c669c39),
    ("band_width_step/false", 0x2269d4469f6d3031),
    ("band_four_way/false", 0xa924a30d7cce27a4),
    ("band_pass_through/false", 0x8bab719977233f20),
    ("width_pass_through/false", 0xed996583094f5512),
    ("width_bridge_pass_through/false", 0x89e9262a67209a4c),
    ("bridge_t/false", 0xc883e1f1c99ca933),
    ("bridge_bend/false", 0x8068311a7e637744),
    ("bridge_pass_through/false", 0x9824ea1e7f137ba0),
    ("bridge_rural_t/false", 0x2c3f444e1d36ec83),
    ("tunnel_t/false", 0xd8ec837c4448802f),
    ("tunnel_bend/false", 0xa50d2f231833253f),
    ("tunnel_pass_through/false", 0x303c028108500646),
    ("band_step_curved/false", 0x37629ac5df89e879),
    ("band_width_step_curved/false", 0xd3bc48a4d792eb19),
    ("t/true", 0x8e5e0a77ca65c074),
    ("four_way/true", 0xb041eb300213983b),
    ("five_way/true", 0xb83bbda1b7627fae),
    ("six_way/true", 0x43c79827aaaf9977),
    ("y_25/true", 0x45e956ad188e4793),
    ("y_40/true", 0x60c6770b5d7fb8bd),
    ("y_60/true", 0x4c844b6123f65e3f),
    ("bend_10/true", 0xe85cc1400c6612e8),
    ("bend_30/true", 0xdc607916d5240b54),
    ("bend_50/true", 0xaaa36e92e23d4b36),
    ("bend_70/true", 0x0f0d7aa790318a32),
    ("bend_90/true", 0xcb014c2008c6a6cc),
    ("bend_110/true", 0x17223cb4f3fe1edf),
    ("bend_130/true", 0x16f1ce739940280b),
    ("bend_150/true", 0x2e01da7d4d05dc1f),
    ("bend_160/true", 0x531cef57fd8d8083),
    ("mixed_40/true", 0x25f61e35da006bbe),
    ("mixed_60/true", 0x6aa8aeb1d114ac6a),
    ("mixed_90/true", 0xbd1387e3c204942e),
    ("width_step/true", 0xe41dcc2e3458c740),
    ("width_four_way/true", 0xf6ef6eb2e505bed3),
    ("width_bend/true", 0xe5f2fea8feb96784),
    ("width_step_convex/true", 0x76f1a02105e2860a),
    ("width_step_miter/true", 0x829db9855f7020aa),
    ("straight_convex/true", 0x8c6b662e2ffd5fc8),
    ("width_step_curved/true", 0x2d854ac11e2299e7),
    ("t_corner/true", 0x90723a04d52c7376),
    ("t_beside_arm/true", 0x6902c4ba37a87f55),
    ("t_beside_arm_shallow/true", 0x8934498c8e53253b),
    ("road_end/true", 0xbaccf81bed76d036),
    ("road_end_side/true", 0x593906a0889ee4c4),
    ("road_end_both_sides/true", 0xca395558d7069530),
    ("through_road/true", 0x6da947d4a9bf694c),
    ("bend/true", 0x737fdac3a9d332a5),
    ("path_end/true", 0x707032df8e2178a1),
    ("path_bend/true", 0xbad3c7fc36956c52),
    ("path_t/true", 0x3a722a8a8892de64),
    ("pass_through_north_south/true", 0xb7fb3bda898b1659),
    ("pass_through_south_north/true", 0x4d00bde0841481a1),
    ("pass_through_bent/true", 0x11f7ac1b667b8c7e),
    ("t_junction/true", 0xbf9842a3adcc264e),
    ("mixed_profiles/true", 0x934042eac0940c53),
    ("bend_15_120/true", 0x4f5af8b174f39f2b),
    ("rural_t/true", 0x998efb476023843d),
    ("rural_four_way/true", 0x0440935ab1ca8a1c),
    ("rural_y_40/true", 0xcc61581f9f866568),
    ("rural_bend_90/true", 0x748c0d66d2ce55f5),
    ("rural_bend_150/true", 0x74a071a20c0b16b5),
    ("rural_side_road/true", 0x4642107ee438a967),
    ("rural_through_road/true", 0x162c16246f46a3e6),
    ("rural_wide_t/true", 0x461cf9e6b8a882fc),
    ("rural_bend_mixed/true", 0x4c176a94cd8d54d1),
    ("band_step/true", 0xa78be6b79a5d85ca),
    ("band_width_step/true", 0x5cbfb099a1464442),
    ("band_four_way/true", 0xa522d71baf1fd2a3),
    ("band_pass_through/true", 0x4b2a4edda48c1540),
    ("width_pass_through/true", 0x7f273f7e88ee3dae),
    ("width_bridge_pass_through/true", 0x52ac105760c28384),
    ("bridge_t/true", 0xfd32193f4459827c),
    ("bridge_bend/true", 0x5540871d05b18b4c),
    ("bridge_pass_through/true", 0x1ee6bfde0c4115cb),
    ("bridge_rural_t/true", 0xabfb72cd77fd04ee),
    ("tunnel_t/true", 0xe4b18086f4548d51),
    ("tunnel_bend/true", 0x104995dd584ad084),
    ("tunnel_pass_through/true", 0x51e1a348b4ccbc48),
    ("band_step_curved/true", 0x023fc8a5bc92c4b7),
    ("band_width_step_curved/true", 0x673bc994f267b9d6),
    ("short_link_10", 0x0288e54ed509e89b),
    ("short_link_13", 0xbc2702dd3de9027e),
    ("short_link_16", 0xd36041e9065690f6),
    ("short_link_20", 0x956ca798931c77f3),
    ("short_link_25", 0x545a60317e3c77bf),
    ("short_link_30", 0x5875266d8a2ea949),
    ("short_link_40", 0xed90621b256b52af),
];
