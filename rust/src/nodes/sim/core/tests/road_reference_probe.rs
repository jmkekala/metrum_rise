// SPDX-License-Identifier: GPL-3.0-or-later

//! The roadbed product probe over the rendered reference junctions, committed stroke by stroke
//! through `SimCore::commit_road` as the reference harness does, flat and on its 2.5% slope: graph
//! profiles, node and span pieces, layouts, walkways and lanes hash exactly as recorded.

use super::road_edit_order::{ARM_M, Stroke, committed};
use crate::simulation::network::roadbed::fixtures::{ProductProbe, assert_probes};
use godot::prelude::Vector2;

// The reference harness's fixtures, by name: strokes and their commit order.
pub(super) fn reference_fixtures() -> Vec<(&'static str, Vec<Stroke>, Vec<usize>)> {
    let road = |a: Vector2, b: Vector2| ([a, b], 1);
    let west_east = road(Vector2::new(-ARM_M, 0.0), Vector2::new(ARM_M, 0.0));
    let side = road(Vector2::ZERO, Vector2::new(0.0, ARM_M));
    let stem = road(Vector2::new(-ARM_M, 0.0), Vector2::ZERO);
    let mut set = vec![
        ("t", vec![west_east, side], vec![0, 1]),
        (
            "four_way",
            vec![
                west_east,
                road(Vector2::new(0.0, -ARM_M), Vector2::new(0.0, ARM_M)),
            ],
            vec![0, 1],
        ),
    ];
    // The Y at `ROAD-54`'s arm limit (20 degrees since `ROAD-126`; its `f32` points read 19.999998
    // degrees, within the arm angle's rounding) and at the old one, each also drawn branches first:
    // at the limit in every orientation (`ROAD-68-K23`).
    for (name, angle_deg, branches_first) in [
        ("y_20", 20.0_f32, Some("y_20_branches_first")),
        ("y_25", 25.0, Some("y_25_branches_first")),
        ("y_40", 40.0, None),
        ("y_60", 60.0, None),
    ] {
        let half = (0.5 * angle_deg).to_radians();
        let strokes = vec![
            stem,
            road(Vector2::ZERO, Vector2::from_angle(half) * ARM_M),
            road(Vector2::ZERO, Vector2::from_angle(-half) * ARM_M),
        ];
        if let Some(branches_first) = branches_first {
            set.push((branches_first, strokes.clone(), vec![1, 2, 0]));
        }
        set.push((name, strokes, vec![0, 1, 2]));
    }
    let bend = road(
        Vector2::ZERO,
        Vector2::from_angle(45_f32.to_radians()) * ARM_M,
    );
    set.push(("bend_45", vec![stem, bend], vec![0, 1]));
    set.push(("terminal", vec![stem], vec![0]));
    let wide = ([west_east.0[0], west_east.0[1]], 2);
    set.push(("t_mixed_4lane_through", vec![wide, side], vec![0, 1]));
    let footpath = (
        [
            Vector2::ZERO,
            Vector2::from_angle(120_f32.to_radians()) * 60.0,
        ],
        0,
    );
    set.push((
        "t_footpath_beside_arm",
        vec![west_east, side, footpath],
        vec![0, 1, 2],
    ));
    set
}

#[test]
fn reference_junction_products_match_the_probe() {
    let mut actual = Vec::new();
    for sloped in [false, true] {
        for (name, strokes, order) in reference_fixtures() {
            let core = committed(&strokes, &order, sloped);
            let network = &core.transit_network;
            let probe =
                ProductProbe::of(&core.region_graph, &network.roadbed, &network.lane_system);
            actual.push((format!("{name}/{sloped}"), probe));
        }
    }
    assert_probes(&actual, EXPECTED);
}

// The crossing kerb ends (`ROAD-68-K20`): every crossing connector's kerb ends, the marking's
// kerb segment, stand on the carriageway (D8, the crossing contract), at its height a centimetre
// inside the crossing and below the curb's top a centimetre outside it. Every reference junction,
// flat and sloped, in each quarter turn and mirror of its strokes: rounding puts the ends a hair
// on either side of the curb line differently in each, and the surface must not follow it. Each
// orientation also plans and commits every stroke (`ROAD-68-K23`): the 25° Y, then exactly at the
// illegal arm angle, was refused in two of them when the angle was measured from each arm's own
// `atan2`.
#[test]
fn crossing_kerb_ends_stand_on_the_carriageway_in_every_orientation() {
    // Exact in f32; `+ 0.0` keeps a negated zero coordinate positive.
    let turns: [fn(Vector2) -> Vector2; 8] = [
        |p| p,
        |p| Vector2::new(-p.y + 0.0, p.x),
        |p| Vector2::new(-p.x + 0.0, -p.y + 0.0),
        |p| Vector2::new(p.y, -p.x + 0.0),
        |p| Vector2::new(-p.x + 0.0, p.y),
        |p| Vector2::new(p.y, p.x),
        |p| Vector2::new(p.x, -p.y + 0.0),
        |p| Vector2::new(-p.y + 0.0, -p.x + 0.0),
    ];
    let mut ends = 0;
    for sloped in [false, true] {
        for (name, strokes, order) in reference_fixtures() {
            for (t, turn) in turns.iter().enumerate() {
                let turned: Vec<Stroke> = strokes
                    .iter()
                    .map(|&(points, lanes)| (points.map(turn), lanes))
                    .collect();
                let case = format!("{name}/{sloped} turn {t}");
                let core = committed(&turned, &order, sloped);
                let network = &core.transit_network;
                let (lanes, roadbed) = (&network.lane_system, &network.roadbed);
                for &id in lanes.node_lanes.values().flatten() {
                    if lanes.crosswalk(id).is_none() {
                        continue;
                    }
                    let g = &lanes.lanes[id].geometry;
                    let across = Vector2::new(g[2].x - g[1].x, g[2].z - g[1].z).normalized() * 0.01;
                    for (end, inward) in [(g[1], across), (g[2], -across)] {
                        let road = roadbed.carriageway_height_at(end.x + inward.x, end.z + inward.y);
                        let curb = roadbed.height_at(end.x - inward.x, end.z - inward.y);
                        let (Some(road), Some(curb)) = (road, curb) else {
                            panic!("{case}: no surface beside the kerb end {end:?}");
                        };
                        assert!((end.y - road).abs() < 0.005, "{case}: {end:?} off the carriageway {road}");
                        assert!(curb - end.y > 0.1, "{case}: {end:?} not below the curb's top {curb}");
                        ends += 1;
                    }
                }
            }
        }
    }
    assert!(ends > 0, "no crossing");
}

// Recorded at `79e7a476` (probe v2: errors by variant and values, layouts field by field) and
// re-recorded by `ROAD-68-D`: every span hash changes with the span outer chains, which now keep the
// road on their left; nodes, layouts, walkways and queries change only where an arm's knot lateral
// differs now that it divides by the segment length instead of multiplying by its reciprocal.
// Re-recorded by `ROAD-68-D` part 3, each change attributed by a build without the other:
// the constructive span station rule changes the spans and queries of 18 junctions (and the lanes
// of the 9 sloped ones; the terminal stubs keep theirs); the exact terminal cap apex then changes
// the nodes, layouts, walkways, lanes and queries of all 20, never the spans. No change touches
// the graph. Re-recorded by `ROAD-68-E` part 2, each change attributed by a build without the
// next: open question 16 changes only `y_25/true` (graph and every piece product but the
// layouts), whose last branch no longer pins to the first branch's span beside the node; open
// question 30 (a) then changes the five sloped fixtures whose second stroke starts at a road end
// (`y_25_branches_first`, `y_25`, `y_40`, `y_60` and `bend_45`, the same products), because that
// stroke no longer re-solves the road it extends. Flat fixtures and the other sloped ones hash as
// before.
// Re-recorded by `ROAD-68-F` part 2: `y_25`'s lanes alone change, flat and sloped. Its second
// stroke leaves a 167.5 degree bend whose arm without the crossing ends its corner 0.77 m out,
// inside `s_min`; the minimum run beyond a corner moves that mouth to 1.77 m (was 1 m), and the
// lanes that commit builds keep their ids in the final Y. Its graph, pieces, layouts, walkways and
// queries are unchanged, as is every other entry.
// Re-recorded by `ROAD-68-E` part 2 round 3 and, rebased onto `ROAD-68-F` part 2, by round 4,
// attributed by a build of the combined tree with question 31's sample reverted, which gives
// `ROAD-68-F` part 2's table exactly: open question 31's plane sample at the mouth station
// where it lies beyond 12 m changes the four sloped Y fixtures (`y_25_branches_first`, `y_25`,
// `y_40`, `y_60`; graph, nodes, spans, walkways, lanes and queries, not the layouts), whose
// narrow-angle mouths lie beyond 12 m; the plan-excerpt fix of open question 8 changes none.
// `y_25/true`'s lanes differ from both parents' tables: `ROAD-68-F`'s minimum run moves the
// intermediate bend's mouth, question 31 the Y's plane, and the lanes follow both.
// Re-recorded by `ROAD-68-K19` (the exact, closed coverage predicate): the lanes of 14 fixtures
// change (not `four_way`, `terminal` or `t_mixed_4lane_through`), and `t_footpath_beside_arm`'s
// queries, flat and sloped; no graph, piece, layout or walkway hash. Of the 197,530 predicate
// calls the fixtures make, 37,953 differ because the old band held a point outside the closed
// triangle and 16 because a triangle under 0.0005 m^2 now holds one; none by arithmetic. No lane
// point's height turns `None` or `Some`. The lane points that move (968 of 79,527 evaluations
// over every lane sync) move by 0.12 m: junction foot-lane points at a crossing's kerb, a hair
// outside the sidewalk's triangles, which the band lifted to the curb's top and which stand on the
// carriageway (`roads.md` question 37, `ROAD-68-K19`). One query answer turns `None`.
// Re-recorded by `ROAD-68-K20` (crossing kerb ends on the carriageway): the lanes of the 16
// fixtures with a junction crossing change (not `bend_45`, whose crossing ends already stood on
// the carriageway, or `terminal`), flat and sloped; no graph, piece, layout, walkway or query
// hash. Against the lane sync before (every point by coverage), the only live-lane points that
// differ are crossing connectors' kerb ends, each 0.12 m lower: 4 to 16 of the 12 to 16 kerb
// ends of each fixture, those a hair inside the curb's top or on its line, which now take the
// published kerb's height on the carriageway.
// Re-recorded by `ROAD-94` (default junction lane use): `t_mixed_4lane_through`'s lanes alone
// change, flat and sloped. Its 2 + 2 through road's approaches now get one connector per lane
// pair the lane use allows instead of every pair, and the 1 + 1 side road's turns fan onto the
// through road's inner or outer lane. Its graph, pieces, layouts, walkways and queries are
// unchanged, as is every entry whose roads have one lane per direction.
// Re-recorded by `TRAFFIC-23` (user decision: permissive default lane use; a one-lane turn group
// fans onto every lane of its target): `t_mixed_4lane_through`'s lanes alone change again, flat
// and sloped (the side road's turns now reach both through lanes, and the through road's
// approaches serve straight from every lane). Every other product and entry is unchanged.
// Re-recorded by `PERF-28` (a local lane rebuild reuses the ids of the retired lanes with the
// rebuilt lanes' identities): the lanes of every fixture but `terminal` change, flat and sloped,
// as the commits leave fewer retired lanes and other ids; no other product. Every live lane is
// unchanged up to its id: an id-free hash of the live lanes (identity, geometry, lengths,
// successors by identity, crossings, ring entries and the owners' lists) is equal to `dev`'s in
// all 20 entries.
// Round JS5f (`ROAD-126`) adds `y_20` and `y_20_branches_first`, flat and sloped; every other entry
// is unchanged.
// Re-recorded when heights became metres (grid alignment A1): the sloped entries' products other
// than the layouts move in their last bits (the slope brush runs in metres); against the control
// every count and road-surface lattice is equal and every height within 2.4e-6 m. The flat
// entries are unchanged.
const EXPECTED: &[(&str, [u64; 7])] = &[
    (
        "t/false",
        [
            0xdcefdb84bc6a950c,
            0x930a59d03d79c762,
            0x2556dcca71547b6f,
            0x47347647c0875527,
            0x4edf89ccac373d17,
            0x01d1a3734ca45e51,
            0x838ac943f4110af7,
        ],
    ),
    (
        "four_way/false",
        [
            0xa1771f3fa157cbf1,
            0x202557ea18699299,
            0x8ac53c6ba6dd075e,
            0x95f31d9618ccf200,
            0xafee7454c4568380,
            0x0e9e0eb903ac3f15,
            0xd99b058cb0fae1a5,
        ],
    ),
    (
        "y_20_branches_first/false",
        [
            0xf4ddc10ddf95ee62,
            0xe08fcb5587e66e55,
            0xfedf5f7663f2638d,
            0x2c098922da49365f,
            0xe9752a961bd2274c,
            0x3b28de6d71923378,
            0x3273ac56f146a9f6,
        ],
    ),
    (
        "y_20/false",
        [
            0x3ae2d28f0a029bda,
            0x5a2cca83c21c307d,
            0x0081d7361ff12349,
            0x1c99560c08df9f52,
            0x9a9ca432eaeb1084,
            0xe7e9a95ab561d926,
            0x59811538437894f5,
        ],
    ),
    (
        "y_25_branches_first/false",
        [
            0x34915670cb723d9f,
            0x4149c69c30755230,
            0xe4c667d40f4b3c30,
            0x944772f28cdd9b4a,
            0x795cbfa449e2504b,
            0x1203b4bed5722227,
            0x58b095a7a059d1e1,
        ],
    ),
    (
        "y_25/false",
        [
            0xe2e14c4095af0a77,
            0xdf7df162db822694,
            0x2658fc21c04f8df4,
            0xba1a4d470645c172,
            0x5789c0832814c8bf,
            0xb3e9aa98b917b278,
            0x39761dd71a7b771a,
        ],
    ),
    (
        "y_40/false",
        [
            0xf2e690399c7a61f5,
            0xa6878f53e0c7d309,
            0x282bbc68e8e7ad2b,
            0x91b62e25cf8927ab,
            0x17b511e266ee0972,
            0xe425c2fd797c4780,
            0x0d61a4b71492ae0e,
        ],
    ),
    (
        "y_60/false",
        [
            0xaac61b1e4b2f547a,
            0x3d82b4d74ebc43ca,
            0x2108b450ccc98488,
            0x400de8cd0dd99a81,
            0xa406092b01d0389f,
            0x5590c87aad4d5820,
            0x528dcfcd6f252f7c,
        ],
    ),
    (
        "bend_45/false",
        [
            0x79029c3f9f08f2d3,
            0xafad50df4fe5716c,
            0xd79b65b1ddcd55c9,
            0x788f7c088a440397,
            0x36d1980fd4be934f,
            0x860803acfe0d60f5,
            0xca3e234a169d1b8e,
        ],
    ),
    (
        "terminal/false",
        [
            0xfac75290dae60182,
            0x8bbb61a081b23daf,
            0x59279120e0cdaf7d,
            0x37aee90de597a1c0,
            0x67570f4a95dc1e84,
            0x7499616c537ed559,
            0xa1027c12245eb089,
        ],
    ),
    (
        "t_mixed_4lane_through/false",
        [
            0x29225eb32b5ed28a,
            0x401ac8cfb475f38d,
            0x8e9b1f1be0bee144,
            0x48243482aceceb3e,
            0x725ba4ed67ab8533,
            0x1902ccd67880faea,
            0x3d058890d0738383,
        ],
    ),
    (
        "t_footpath_beside_arm/false",
        [
            0x1793286afd4aa67a,
            0xc2ae80fbf0332ea8,
            0x1f9eafee9c31ca22,
            0x097a57a202a87c0c,
            0x3e532fb50f5548c7,
            0x86d79a012f767967,
            0x3261c5eaaa8cd8f5,
        ],
    ),
    (
        "t/true",
        [
            0x173b5391aa94c107,
            0x672535b554bd6a93,
            0x2556dcca71547b6f,
            0x5f98e43b38c7abea,
            0x690577ada42b8c92,
            0x8795f1cab591bfaa,
            0x92660b1a29fda08a,
        ],
    ),
    (
        "four_way/true",
        [
            0x6d6c870be0baa7db,
            0x07084823f6681f8a,
            0x8ac53c6ba6dd075e,
            0x5c84ab5dfa040518,
            0xe929351a547e5cd2,
            0x7aa460694f3407f4,
            0x5e4d0fd852b5ea69,
        ],
    ),
    (
        "y_20_branches_first/true",
        [
            0x666ab5bfc099754c,
            0x7a855e46ce52d716,
            0xfedf5f7663f2638d,
            0xa553745d1f3f7283,
            0x487876bb0a1de5ec,
            0xef6cfd44bb9d0ac0,
            0x8249ec3106dd8a83,
        ],
    ),
    (
        "y_20/true",
        [
            0x6e8320ddea0e3818,
            0x3ca760f598262422,
            0x0081d7361ff12349,
            0x876009b8afa48b16,
            0x9bb75ffc073cc844,
            0x9c9b48f0873c3712,
            0x0f3ec4459f7e8ce3,
        ],
    ),
    (
        "y_25_branches_first/true",
        [
            0xde5dbaf79caed11b,
            0x814e594a7300cecd,
            0xe4c667d40f4b3c30,
            0xbe3ab4b2743317d3,
            0x05c448314273e5bb,
            0x4fc689a26cefeb16,
            0x3b109117040b0235,
        ],
    ),
    (
        "y_25/true",
        [
            0x489f4fc6444139d3,
            0x44c4a54a08c4b7b9,
            0x2658fc21c04f8df4,
            0x09f9762c09176cbb,
            0x0b5814af50eb4883,
            0x8fd8f01dec0fe229,
            0xec26d8d576dbb05d,
        ],
    ),
    (
        "y_40/true",
        [
            0xcb4a346c753a7143,
            0x9a343eab50ea21f0,
            0x282bbc68e8e7ad2b,
            0x761ca5d3e9f9bb8e,
            0x1607c131b9a41a5f,
            0xdef2f5870bb1e671,
            0xd12639aac8c0be77,
        ],
    ),
    (
        "y_60/true",
        [
            0x6c87c221895d6b76,
            0x2ff6d6e26b0f4be9,
            0x2108b450ccc98488,
            0x3282907041db795f,
            0x366b292720177c38,
            0xe92b2cbd4d3a4a38,
            0x3dcf9b28c9db9c96,
        ],
    ),
    (
        "bend_45/true",
        [
            0x4e14bb57aa001551,
            0x6cf0fee87854ede2,
            0xd79b65b1ddcd55c9,
            0x1548d2d42317ecc8,
            0x5fe9d919f5b7d943,
            0x60eb6644a4410882,
            0x367b30caf7649721,
        ],
    ),
    (
        "terminal/true",
        [
            0x959aa2eb96fe4be0,
            0x4d76d9f39c86014d,
            0x59279120e0cdaf7d,
            0xc6e6b24fde9a3b79,
            0xd49c950a84ebbcbf,
            0x103bc7ef190bf36f,
            0x25117a26d0334f96,
        ],
    ),
    (
        "t_mixed_4lane_through/true",
        [
            0xe34504b00d77da59,
            0x4122e0659c2981f9,
            0x8e9b1f1be0bee144,
            0x9918f11c9b265283,
            0x5bc25267c14af560,
            0xbcdd67501d4fd0dc,
            0x2957c4c737a94631,
        ],
    ),
    (
        "t_footpath_beside_arm/true",
        [
            0xd92db35827b24d4b,
            0xae8a902e8c8a401e,
            0x1f9eafee9c31ca22,
            0x420c830e59a6fcb7,
            0x2641727bf923b35d,
            0xb0bc246c6a5a4353,
            0x8b5bb17be26c268e,
        ],
    ),
];
