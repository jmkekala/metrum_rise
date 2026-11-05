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
    // Turn pockets (`ROAD-95`), recorded in round R5a2.
    use crate::simulation::network::graph::SidewalkSides;
    use crate::simulation::network::roadbed::fixtures as pockets;
    for (name, graph) in [
        ("t_right_pocket", pockets::t_right_pocket()),
        ("one_way_right_pocket", pockets::one_way_right_pocket()),
        ("two_plus_one_right_pocket", pockets::two_plus_one_right_pocket()),
        ("straight_1x1_to_2x2", pockets::straight_1x1_to_2x2()),
        // Round R5b1: a left pocket narrowing a divided road's median.
        (
            "four_way_2x2_left_pocket",
            pockets::four_way_2x2_left_pocket(),
        ),
        // Round R5b1a: a right pocket on a divided road, alone and beside a left one.
        (
            "four_way_2x2_right_pocket",
            pockets::four_way_2x2_right_pocket(),
        ),
        (
            "four_way_2x2_left_right_pocket",
            pockets::four_way_2x2_left_right_pocket(),
        ),
        // Round R5c1: left pockets on undivided roads, alone and beside a right one.
        (
            "four_way_1x1_left_pocket",
            pockets::four_way_1x1_left_pocket(),
        ),
        (
            "four_way_1x1_left_right_pocket",
            pockets::four_way_1x1_left_right_pocket(),
        ),
        (
            "undivided_2x2_left_pocket",
            pockets::undivided_2x2_left_pocket(),
        ),
        (
            "undivided_2x2_left_right_pocket",
            pockets::undivided_2x2_left_right_pocket(),
        ),
        // Round S2: a right pocket on a one-sided road, on its shoulder side and its sidewalk's.
        (
            "t_right_pocket_left_sidewalk",
            pockets::t_right_pocket_one_sided(SidewalkSides::LEFT),
        ),
        (
            "t_right_pocket_right_sidewalk",
            pockets::t_right_pocket_one_sided(SidewalkSides::RIGHT),
        ),
    ] {
        set.push((name.to_string(), graph));
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
            check_abutments(&name, &graph, span, edge);
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

// `TERRAIN-07-C` (T22 4 (a)): a bridge span hangs one abutment face (two triangles) at each end
// whose node another standard edge meets, so a grounded piece shares its mouth row; no other
// span hangs one. Its top points are the fascias' bottoms below the row's two corners (the
// slab's end edge, bitwise), its bottoms the skirt depth below them (the grounded skirts'
// bottoms), and it faces the space under the bridge, into the span.
fn check_abutments(name: &str, graph: &RegionGraph, span: &SpanPiece, edge: usize) {
    let mesh = span.mesh();
    let mut faces = Vec::new();
    mesh.for_each_abutment_triangle(|points, bottoms| faces.push((points, bottoms)));
    let record = graph.edge(edge);
    let meets_ground = |node: u32| {
        let node = graph.get_valid_node(node);
        graph.node_adjacency(node).iter().any(|&other| {
            other != edge
                && graph
                    .get_edge(other)
                    .is_some_and(|other| !other.deleted && other.class == EdgeClass::Standard)
        })
    };
    let ends: Vec<&MouthSection> = [
        (record.start_node, span.start_mouth()),
        (record.end_node, span.end_mouth()),
    ]
    .into_iter()
    .filter(|&(node, _)| record.class == EdgeClass::Bridge && meets_ground(node))
    .map(|(_, mouth)| mouth)
    .collect();
    assert_eq!(
        faces.len(),
        2 * ends.len(),
        "{name} span {edge}: abutment faces"
    );
    let positions = mesh.positions();
    let centroid = positions.iter().copied().sum::<RoadVec3>() / positions.len() as f64;
    for (points, bottoms) in faces {
        for (point, bottom) in points.iter().zip(bottoms) {
            let kind = if bottom {
                StripKind::Skirt
            } else {
                StripKind::Fascia
            };
            let below_a_corner = ends.iter().any(|mouth| {
                let row = &mouth.vertices;
                [row[0], row[row.len() - 1]]
                    .iter()
                    .any(|&corner| bits(kind.bottom(corner)) == bits(*point))
            });
            assert!(
                below_a_corner,
                "{name} span {edge}: an abutment vertex off its corners"
            );
        }
        let [p, q, r] = points;
        let normal = -(q - p).cross(r - p);
        assert!(
            normal.dot(centroid - (p + q + r) / 3.0) > 0.0,
            "{name} span {edge}: an abutment faces away from the bridge"
        );
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
// Re-recorded by `ROAD-68-K19` (the exact, closed coverage predicate): `bend_10` changes its
// queries alone, flat and sloped: at one probe point just outside every carriageway triangle
// (inside the old predicate's band) the carriageway and carriageway-only lane queries answer
// `None`; no answer turns `Some`, and every other answer is bitwise equal. Attributed by
// classifying every predicate call against the old predicate: 24 of 437,772 differ, all band
// points. No piece, layout, walkway or lane hash changes.
// Re-recorded by `ROAD-61` round R8b: a lined piece's bounds hold its lining (the headwalls'
// wings reach past the deck), so the six tunnel entries' spans change, and with them the queries
// of those whose top-triangle walk over the piece's bounds meets more triangles. No other entry
// changes.
// Re-recorded by `ROAD-61` round R8b's lining shell: its feet stand the thickness outside the
// walls, so where they reach past the deck and the wings the bounds, and with them the spans and
// queries, change again: `tunnel_bend` and `tunnel_pass_through`, flat and sloped. No other entry
// changes.
// Re-recorded by `ROAD-61` round R8f: the collar replaces the headwall and its wings, so the six
// tunnel entries' bounds shrink to the shell, and with them their spans and queries (the
// pass-throughs' spans alone). No other entry changes.
// Re-recorded by `ROAD-94` (default junction lane use): the lanes of the 117 entries with a road
// of more than one lane per direction change (the fixtures' 7 m and 14 m one-way roads carry two
// and four lanes), and nothing else: their junctions now give each approach lane its movements
// and fans, and their degree-two nodes continue each lane on the lane of its own rank. The 24
// entries whose roads have one lane per direction hash as before, flat and sloped.
// Re-recorded by `TRAFFIC-23` (user decision: permissive default lane use, every approach lane
// goes straight where straight exists; a one-lane turn group fans onto every lane of its target):
// the lanes of 67 entries change, all junctions of three or more arms with a multi-lane road, and
// nothing else. Bends, pass-throughs, road ends and the junctions whose masks and fans are as
// before (77 entries) hash as before.
const EXPECTED: &[(&str, [u64; 7])] = &[
    (
        "t/false",
        [
            0x3f2ce5378110f69f,
            0xd2c253f19b1ef6da,
            0xacdced0eba6074f4,
            0x99ed70f1e1d83e4e,
            0xb69651d1d18119a9,
            0x188ec67ebf716df8,
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
            0xc71488f03418744d,
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
            0xfc0358a7816c2bcd,
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
            0x599ef67c2f8cde1b,
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
            0x6c6088a5bc7d209d,
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
            0x88fd2f59e51b6e67,
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
            0x6f176a6b4c261ed6,
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
            0x5ef2e90309f943dd,
            0xde3af4121d8f1e38,
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
            0x8167a3024fcb2f12,
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
            0x19246c096678a2ef,
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
            0x9bf33562e72e4cdc,
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
            0xc600c36749874610,
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
            0x89607f76a0e5daae,
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
            0x101bd7a398210c13,
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
            0x0e4ec3cfcc1e3709,
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
            0x805918b561a5d495,
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
            0x4ccd8cf965549e8f,
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
            0x37d39cb3da75cad9,
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
            0x27156a4bd47e328d,
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
            0x71cebaf464fd38b1,
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
            0x71b47ed5cea3677f,
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
            0x9574eda43ba78ba0,
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
            0x7f3f8ec0f80bc2e8,
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
            0x6ee0ed3a91ebbcb1,
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
            0xd5ace0bb99491ab1,
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
            0x54ebee008102dd1b,
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
            0x7ffead5ef544fe04,
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
            0xef747952eb8d0800,
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
            0x631c6f796173399e,
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
            0x471886dcf56f16bc,
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
            0x14f03a394685a3fb,
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
            0xcce27d9bab273f4b,
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
            0xba09b24cd82eff10,
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
            0xca49b6ad53c82b26,
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
            0x76421751fbb98c1a,
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
            0xaf9130a2fa2ef9ef,
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
            0xe3b64c94e9b8f5e6,
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
            0x85a4dfeaabcc1f82,
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
            0x8165a181b2348592,
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
            0x33534f71022e67ba,
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
            0xb8cb0fd258429be0,
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
            0x6ef55b3a4ca415f9,
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
            0xa279652a572d1419,
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
            0xde3811dbe675922e,
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
            0x2d20a5c11a2e7bc1,
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
            0xf27ca68cb8c6e900,
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
            0x86d9428035ff204f,
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
            0xf3e24b22751a67b0,
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
            0xae60229ba1930cfe,
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
            0xbde39748c5dc7bab,
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
            0x1d6d26311ce49d5a,
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
            0xb0406e7130c77e68,
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
            0x5723151c04b1b779,
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
            0x217d8c7774a51e29,
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
            0x933281f1f69affcd,
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
            0xdf06b74f6ac03642,
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
            0xaeb43935556b2328,
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
            0x35134acc32319b43,
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
            0x66c22f5241d8fd05,
            0x9ba901a1454c6ef8,
        ],
    ),
    (
        "tunnel_t/false",
        [
            0xc7e956e72dd72ebf,
            0x316cd30bc4c6d451,
            0x6916fbee27fd7d64,
            0x7cad2997706c80c5,
            0x38af174d571d9151,
            0xf4f6e09d7e207e42,
            0x8e7bf2e23f88088c,
        ],
    ),
    (
        "tunnel_bend/false",
        [
            0xe9ee337eaf5047cd,
            0xda08c0c406e26bbf,
            0xc3bd732dc7c3d626,
            0x868b1da6a7d00b2d,
            0x47b9e4ef0180be6c,
            0x2b29657c8eeb3de8,
            0x1c660fdb6f0efe0b,
        ],
    ),
    (
        "tunnel_pass_through/false",
        [
            0x4f05dc7283f3782d,
            0x07579bad55785ce6,
            0xa56e334196ab5f26,
            0x3d573c6083f9cd81,
            0xb453e5bd6aa15521,
            0x38b02b33e213a121,
            0x808d87d84942f9b0,
        ],
    ),
    // One-sided sidewalks (`ROAD-107`), recorded by round S1: new entries, every other entry
    // unchanged.
    (
        "one_sided_pass_through/false",
        [
            0x758373470f12dfa5,
            0xcf281781e9d1ea23,
            0x483511bc262bb349,
            0x78d90ef1105b45b4,
            0x4add7030300964c3,
            0x7b7c50191a04561e,
            0x21bfca86d2695d12,
        ],
    ),
    (
        "one_sided_stem_left/false",
        [
            0x5adec4ac0a31114a,
            0x7ffd81f478e91d9c,
            0xf07e0234b55e891e,
            0xf1c91bbc222978ba,
            0xc9ff17e549216d13,
            0x73ddd15760af336e,
            0x98b1776250e6c671,
        ],
    ),
    (
        "one_sided_stem_right/false",
        [
            0x5adec4ac0a31114a,
            0x3a6c1916f55c1f53,
            0x1a0cace1d914d856,
            0xb094cea5a8f965de,
            0x14a38b96b95be1c1,
            0x649193ec87f5415d,
            0x3093586c214290b9,
        ],
    ),
    (
        "one_sided_continues_two_sided/false",
        [
            0xfbd01f1fb25b829c,
            0x68749e0770e69c8a,
            0xc447508d074769f5,
            0xba1e17f605fa1a89,
            0x95d6c6bc5dba9b4f,
            0x1f05f179e21ca83a,
            0xa8360fbf3993d977,
        ],
    ),
    (
        "one_sided_one_way_t/false",
        [
            0x5adec4ac0a31114a,
            0x3a6c1916f55c1f53,
            0x1a0cace1d914d856,
            0xb094cea5a8f965de,
            0x14a38b96b95be1c1,
            0x419d7f043cb69c33,
            0x3093586c214290b9,
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
            0x17ae9b15425e663b,
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
            0x41caa690d9021b5a,
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
            0x8b6b1389ebbb3489,
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
            0xb0632c4a76475d0e,
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
            0xb73773efab8ed644,
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
            0x30dd47403e0d4676,
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
            0x675b60777decc62d,
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
            0x5d04e34aab17e7e9,
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
            0xb94356d8951c76bf,
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
            0x9870db3190c1e3d0,
            0xf5cafb41792b0784,
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
            0x40ff2805bc16c2de,
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
            0xf3e681d99e7ada3c,
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
            0x24985066c4bd7492,
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
            0x06836aa3a508a011,
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
            0x0d6005918ae2a07f,
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
            0x160beadd19f74516,
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
            0x6de30fc584b25835,
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
            0x58530be4465e7e92,
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
            0xb73caa6f687745e6,
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
            0xfbae9ea5c0ed8f0d,
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
            0xe26a6213099c4fa7,
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
            0x8dc01a8743c3f9d6,
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
            0xd9be355777c88353,
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
            0x149cb3140d8b2758,
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
            0x626d5df6fd91ce6f,
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
            0x8500d180042cc485,
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
            0x6f535415f889e450,
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
            0xca81d09d4dfe95e9,
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
            0xd1a76d5fe09d88d9,
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
            0xfba6ad22a06a1f11,
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
            0xb8e99162e822b3be,
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
            0x0be963efabd9a589,
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
            0x4954d40fe30a4c71,
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
            0xd3cfd8a464eea10e,
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
            0xe180bd4a5ed11030,
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
            0x7acb57ffa6a13c8d,
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
            0x81eb82b008400df0,
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
            0x03b0144025eb3eb1,
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
            0x10e22377606a30ed,
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
            0xc7c6a5a1fa3a38ad,
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
            0x579f81019db150a4,
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
            0xde64c4912c742dd7,
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
            0x067169f84fac06db,
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
            0x6356927e337277b0,
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
            0xc6c460e24ccf5549,
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
            0x869ca0a703c28aef,
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
            0x94f52b8d529bfc2e,
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
            0x4abd257a2b4703ea,
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
            0x0d0ca834124a8d26,
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
            0x0d9b89e7bf977bc5,
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
            0xf19a3fe0c9900d4b,
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
            0x591197782e2c4144,
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
            0x750ebdb3f260a103,
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
            0x1521112ff90bc9b5,
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
            0x93b7d8f1dd59123f,
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
            0x24c71eb652b15d85,
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
            0x79f0cfb1395cbc55,
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
            0xbabe38ca6501e5dd,
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
            0xf52b8ac68a941641,
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
            0xc7d301452d61d909,
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
            0xbe73e1db46c8cf00,
            0x976922a1d3f99cff,
        ],
    ),
    (
        "tunnel_t/true",
        [
            0x522ec5c67b80d40e,
            0x1ddcb9d3a4cff34d,
            0x6916fbee27fd7d64,
            0xfabed6fdceace920,
            0x809c7d55d4aaeb50,
            0x9cec2974f26f4259,
            0x0bfe5f41c78bc114,
        ],
    ),
    (
        "tunnel_bend/true",
        [
            0x5bb9074887d9d00a,
            0x605b0e92a436dba2,
            0xc3bd732dc7c3d626,
            0xcbb6be8bc8d90b74,
            0x783a19c1f4b3a36f,
            0x8ea7e2c0d5e398bf,
            0x6d9383aaa4548fb9,
        ],
    ),
    (
        "tunnel_pass_through/true",
        [
            0x8315a3446fa6d340,
            0x01a2923d59ff37cc,
            0xa56e334196ab5f26,
            0x792fbd16c812710e,
            0x6cce29be6e4ef515,
            0x8253cf1b11e9681f,
            0x742acff099dc78fa,
        ],
    ),
    (
        "one_sided_pass_through/true",
        [
            0x45fd26e520d97809,
            0x7543800e26b0bdff,
            0x483511bc262bb349,
            0xeccf73687e974853,
            0xa447c3ecdd6f943e,
            0x9ee56d7ebca76dc8,
            0x892e21eb444677b9,
        ],
    ),
    (
        "one_sided_stem_left/true",
        [
            0x86498cd789f973f3,
            0x70cf1682f960b7b9,
            0xf07e0234b55e891e,
            0x4803f2b760de914b,
            0x642740799d2708d1,
            0x0918d7218b0c8a86,
            0x9d0d203585d05b8b,
        ],
    ),
    (
        "one_sided_stem_right/true",
        [
            0x86498cd789f973f3,
            0xf599238e03bb2b76,
            0x1a0cace1d914d856,
            0x945dc053e105f6df,
            0x8cdcbfa8124a76fb,
            0x3d716b25367d7abe,
            0x40dc605da97f1c3a,
        ],
    ),
    (
        "one_sided_continues_two_sided/true",
        [
            0xb39996db3641ae2c,
            0x9ed720c8fd500427,
            0xc447508d074769f5,
            0xe61b18aa317095ed,
            0x3ca3b25f92666498,
            0x8323da71575ad5f4,
            0xf3067f6367c3d030,
        ],
    ),
    (
        "one_sided_one_way_t/true",
        [
            0x86498cd789f973f3,
            0xf599238e03bb2b76,
            0x1a0cace1d914d856,
            0x945dc053e105f6df,
            0x8cdcbfa8124a76fb,
            0xfd30370bd5cdb388,
            0x40dc605da97f1c3a,
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
            0xdc82bbc36ffb666e,
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
            0xff6812a02bec4800,
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
            0xf373eeb919b5e75c,
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
            0xbcdc5728cffcc2a0,
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
            0x43ce2414b12f8408,
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
            0xe9d26706c5ff9b9a,
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
            0x1aca73df16c3b830,
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
            0x4d2b706370213a78,
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
            0xc791bfa1a1e512d8,
            0x87926626f99140ad,
        ],
    ),
    // Turn pockets (`ROAD-95`), recorded by round R5a2: new entries, every other entry unchanged.
    (
        "t_right_pocket",
        [
            0x3afeffb0ee418eb6,
            0x4cf9fe2efb8a7af3,
            0xa3be4007fbf03d83,
            0xaa86d8c750437be6,
            0xe71d5d36c5ee2d60,
            0x22af890eb610c468,
            0xaecd896ccc2f9595,
        ],
    ),
    (
        "one_way_right_pocket",
        [
            0x79e130fc36fd6c4d,
            0x9af782198329d481,
            0x0be5def56aa07b87,
            0xc9242c670a1d85ff,
            0xea0f2f2425f9afce,
            0xeaae0db9437fc779,
            0x9830d3935a933f3f,
        ],
    ),
    (
        "two_plus_one_right_pocket",
        [
            0xc35886d408f7a181,
            0x8393ca9a20a74a9c,
            0x37de5d34931c048a,
            0x23200d6c4947b5db,
            0x423ce45cdac75393,
            0x682a33c3506d5163,
            0xfc2acff9dae4bf48,
        ],
    ),
    (
        "straight_1x1_to_2x2",
        [
            0xa9d0b661f0d6b40b,
            0x92c5b6e5b49386aa,
            0xa9abc111f7609fda,
            0xf72e274d01d23c55,
            0x1ddbab1178d45da1,
            0xf666fd5ed0688037,
            0xea9f1359f25fc3a9,
        ],
    ),
    // Round R5b1: a left pocket in a divided road's median; every other entry unchanged.
    (
        "four_way_2x2_left_pocket",
        [
            0x1646ac8b650ffbeb,
            0x5ff1fb72860d42b4,
            0xe52db5cb4752617b,
            0x124e5c3d50b08925,
            0xb44936f64b416981,
            0x50dce1dcbd02e603,
            0x776be84bcd900cab,
        ],
    ),
    // Round R5b1a: a right pocket on a divided road, alone and beside a left one; every other
    // entry unchanged.
    (
        "four_way_2x2_right_pocket",
        [
            0x20a4e563b7b08db0,
            0x7a8d5dbb8b5abc59,
            0x3fe9d62080f15c39,
            0x622cbec5a9227d92,
            0x27823c59886369fe,
            0xdaa4aa0fac9eaaad,
            0xe9ae01a956a6a50f,
        ],
    ),
    (
        "four_way_2x2_left_right_pocket",
        [
            0x20a4e563b7b08db0,
            0xeb2309920e046275,
            0x3fe9d62080f15c39,
            0xa4b5a67aea7d1547,
            0x27823c59886369fe,
            0x86b23d827ba2f136,
            0x1dcc17848011ca01,
        ],
    ),
    // Round R5c1: left pockets on undivided roads, alone and beside a right one; every other
    // entry unchanged.
    (
        "four_way_1x1_left_pocket",
        [
            0xcc04d2a3b182d420,
            0x4af0a1e7b20ee8ae,
            0xa60b045d508d27ca,
            0x5025ae85ffde7855,
            0xa5b81bea6d87a462,
            0x5a40aaee7c5ffe24,
            0xc837aafee4bf2661,
        ],
    ),
    (
        "four_way_1x1_left_right_pocket",
        [
            0x79f1ed4c2751eb30,
            0x92a54d38ac300ebb,
            0xe70327ca76c7b193,
            0x0c3721b0ccf4ae2e,
            0xa6e420291b72c10e,
            0xe690dc766c651bab,
            0x1cafb03ffde14da3,
        ],
    ),
    (
        "undivided_2x2_left_pocket",
        [
            0x1ed3664ec6ddc130,
            0xb61161fb19f7329d,
            0x7cc3f22d635138b3,
            0xbdef848c470688d8,
            0xefe4792c83846e48,
            0x858fbeb33f8ce384,
            0x4f007c3a1d3b5fc6,
        ],
    ),
    (
        "undivided_2x2_left_right_pocket",
        [
            0xdcc8f37fd58ab470,
            0xa8e78fd08cc57c2c,
            0x2295d104429bf8bb,
            0xa493aa61d3e51433,
            0xd6af7f12ee6c5789,
            0x29c4ae6d2b4a4d1b,
            0xe69e92f15971ed3b,
        ],
    ),
    (
        "t_right_pocket_left_sidewalk",
        [
            0xe8931ceec52ef2c2,
            0x31a36b1ec18a2bc1,
            0xc1ad8444a8dac76e,
            0x400900f9dd455790,
            0x750db5d6ddadcc62,
            0xe01e3cf69058e99f,
            0x32b47541ce3eecd6,
        ],
    ),
    (
        "t_right_pocket_right_sidewalk",
        [
            0xe8931ceec52ef2c2,
            0x4f613ddec8edeedd,
            0xd59c13d4eab2eb17,
            0xb0d3489228048a06,
            0xb2ac3a8c2c375968,
            0x3c2390a9dcd03b23,
            0x9eaefee7e7fb09e4,
        ],
    ),
];

// Recorded by `TERRAIN-07-B`: the strip layer is new, so every entry is new; the seven
// top-surface hashes above are unchanged. Re-recorded by `TERRAIN-07-C` round 2: the 18 entries
// with a structure whose far end is free (the bridge and tunnel families and `mixed_profiles`)
// change with its end cap; every other entry and the top-surface hashes are unchanged.
// Re-recorded by `TERRAIN-07-C` round 4: the ten bridge entries (`bridge_t`, `bridge_bend`,
// `bridge_pass_through`, `bridge_rural_t` and `width_bridge_pass_through`, flat and sloped),
// whose bridge meets a road at its hub end, change with the abutment face hung there; every
// other entry, the tunnels' among them, and the top-surface hashes are unchanged.
// Re-recorded by `ROAD-61` round R8: the six tunnel entries (`tunnel_t`, `tunnel_bend` and
// `tunnel_pass_through`, flat and sloped) change with the lining and headwalls the layer now
// hashes, whose end wall replaces a buried free end's cap; every other entry and the top-surface
// hashes are unchanged.
// Re-recorded by `ROAD-61` round R8b: the same six tunnel entries change with the lining's shell
// (outer walls the thickness outside the walls, a roof the thickness above the ceiling); every
// other entry is unchanged.
// Re-recorded by `ROAD-61` round R8c: the same six tunnel entries change with the headwall's
// wing tops falling at 1:2 to their tips; every other entry is unchanged.
// Re-recorded by `ROAD-61` round R8f: the same six tunnel entries change with the collar in place
// of the headwall and its wings (and the shell's tops computed as the collar's height in one
// step, and a bend's node lining without the triangulator's sliver); every other entry is unchanged.
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
    ("width_bridge_pass_through/false", 0xa061ac08a79003af),
    ("bridge_t/false", 0x52d870975ff21fcd),
    ("bridge_bend/false", 0x4e1d11e189af360f),
    ("bridge_pass_through/false", 0xd8331298dd91b258),
    ("bridge_rural_t/false", 0xd4b525b794ec9395),
    ("tunnel_t/false", 0x9a80bd4325829842),
    ("tunnel_bend/false", 0x25ec9de2f2b9c2df),
    ("tunnel_pass_through/false", 0x35cefc37c2835c7c),
    // One-sided sidewalks (`ROAD-107`), recorded by round S1: new entries, every other entry
    // unchanged.
    ("one_sided_pass_through/false", 0x5e4f0cfe7c09089b),
    ("one_sided_stem_left/false", 0xca0fd295c5363afb),
    ("one_sided_stem_right/false", 0x82d8bc936db2c3e7),
    ("one_sided_continues_two_sided/false", 0x84db0bacb6ef4fda),
    ("one_sided_one_way_t/false", 0x82d8bc936db2c3e7),
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
    ("width_bridge_pass_through/true", 0x116e064cb80cec02),
    ("bridge_t/true", 0x90f933ef44f34eac),
    ("bridge_bend/true", 0x65fa10b8e747d953),
    ("bridge_pass_through/true", 0xf04c12c0e0dd199a),
    ("bridge_rural_t/true", 0x416fcacf8292544e),
    ("tunnel_t/true", 0x325433505d341b02),
    ("tunnel_bend/true", 0x457629880f59b079),
    ("tunnel_pass_through/true", 0xc3ca85f4194972d4),
    ("one_sided_pass_through/true", 0x3c1645f0499fe67f),
    ("one_sided_stem_left/true", 0x58035def016e8a4e),
    ("one_sided_stem_right/true", 0xf70851d7c1c1da34),
    ("one_sided_continues_two_sided/true", 0x7e835e5fe23cffbf),
    ("one_sided_one_way_t/true", 0xf70851d7c1c1da34),
    ("band_step_curved/true", 0x023fc8a5bc92c4b7),
    ("band_width_step_curved/true", 0x673bc994f267b9d6),
    ("short_link_10", 0x0288e54ed509e89b),
    ("short_link_13", 0xbc2702dd3de9027e),
    ("short_link_16", 0xd36041e9065690f6),
    ("short_link_20", 0x956ca798931c77f3),
    ("short_link_25", 0x545a60317e3c77bf),
    ("short_link_30", 0x5875266d8a2ea949),
    ("short_link_40", 0xed90621b256b52af),
    // Turn pockets (`ROAD-95`), recorded by round R5a2.
    ("t_right_pocket", 0xb39623a7c7a3bd70),
    ("one_way_right_pocket", 0x9f0797ef5652d9ca),
    ("two_plus_one_right_pocket", 0xd65835f90817caae),
    ("straight_1x1_to_2x2", 0xf29dd3d5cc0bd09a),
    // Round R5b1.
    ("four_way_2x2_left_pocket", 0x4334955040fd0985),
    // Round R5b1a.
    ("four_way_2x2_right_pocket", 0x3c930dccf14cdd65),
    ("four_way_2x2_left_right_pocket", 0x42b8381b17d3c395),
    // Round R5c1.
    ("four_way_1x1_left_pocket", 0xf09b9185e42d7ca5),
    ("four_way_1x1_left_right_pocket", 0x35e644f19860eced),
    ("undivided_2x2_left_pocket", 0x3c20a6e986fab845),
    ("undivided_2x2_left_right_pocket", 0x6e2d2cf56781779d),
    ("t_right_pocket_left_sidewalk", 0xe78d672c935e5c86),
    ("t_right_pocket_right_sidewalk", 0xe0b57b946ec170d8),
];
