// SPDX-License-Identifier: GPL-2.0-only

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
    for (name, angle_deg) in [("y_25", 25.0_f32), ("y_40", 40.0), ("y_60", 60.0)] {
        let half = (0.5 * angle_deg).to_radians();
        let strokes = vec![
            stem,
            road(Vector2::ZERO, Vector2::from_angle(half) * ARM_M),
            road(Vector2::ZERO, Vector2::from_angle(-half) * ARM_M),
        ];
        if angle_deg == 25.0 {
            set.push(("y_25_branches_first", strokes.clone(), vec![1, 2, 0]));
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
// orientation also plans and commits every stroke (`ROAD-68-K23`): the 25° Y, at exactly the
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
const EXPECTED: &[(&str, [u64; 7])] = &[
    (
        "t/false",
        [
            0xdcefdb84bc6a950c,
            0x930a59d03d79c762,
            0x2556dcca71547b6f,
            0x47347647c0875527,
            0x4edf89ccac373d17,
            0xab1e0c36d04f8f02,
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
            0xf3af16a08a51a56a,
            0xd99b058cb0fae1a5,
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
            0x23d929a664a1913c,
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
            0x7caed2d1f114cf39,
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
            0xf7350e0b15ffbe1b,
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
            0x982049710fc5b88d,
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
            0x37f48709f449746f,
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
            0x0cade73561e84798,
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
            0xb1ae5a23ee631b7a,
            0x3261c5eaaa8cd8f5,
        ],
    ),
    (
        "t/true",
        [
            0xd5b64316ed2a1887,
            0x7b7ac5c649c8b486,
            0x2556dcca71547b6f,
            0xafa5fdd44b1f3e37,
            0xcb3a29371a976116,
            0x9d861caf7b9df84c,
            0x11b90e23664a77c6,
        ],
    ),
    (
        "four_way/true",
        [
            0x3677295d4816a142,
            0xf69c41389020c3d3,
            0x8ac53c6ba6dd075e,
            0x32c7fd63a1eb6355,
            0xc0af058dd2cd34c1,
            0x60a923deaf540896,
            0xd095e221ec6237f7,
        ],
    ),
    (
        "y_25_branches_first/true",
        [
            0xee8d6df479d2e315,
            0xf267bf62f6991cf7,
            0xe4c667d40f4b3c30,
            0x8ff1522bfa09645c,
            0x978ea8db0d82e488,
            0xd9372dff7339389f,
            0xa9dd6d496aa66c76,
        ],
    ),
    (
        "y_25/true",
        [
            0x0ec9605cade18269,
            0x24fe92973bc8d0cf,
            0x2658fc21c04f8df4,
            0x72fb69808e0f820c,
            0x0ff79938b4e00d18,
            0x354915d74535ef8c,
            0x4b4830d61a573246,
        ],
    ),
    (
        "y_40/true",
        [
            0x3bfcdb8984261eec,
            0x0d34fbeec21f73ce,
            0x282bbc68e8e7ad2b,
            0xc3ac97b1b3ac4ecc,
            0x1d1dc420bdc56f63,
            0x7c046fbdd930e403,
            0x0e7813513a04bd2b,
        ],
    ),
    (
        "y_60/true",
        [
            0x268fbb60feaab71b,
            0x0c3034e5bfd7348c,
            0x2108b450ccc98488,
            0xe2969a080ab79204,
            0x8d4202bdb5ac9c55,
            0x884dcf0241695695,
            0x1b69e5524c9a1e0c,
        ],
    ),
    (
        "bend_45/true",
        [
            0xa0dfec84d21d3a6b,
            0x7835b22448a36a92,
            0xd79b65b1ddcd55c9,
            0x0283be9510bd8e7b,
            0x741add2ecb1b7acd,
            0x436555c55884f337,
            0x90190fc393a2b1d3,
        ],
    ),
    (
        "terminal/true",
        [
            0x12b7261362dbb9c6,
            0x6be55a7b695e09ef,
            0x59279120e0cdaf7d,
            0x7063a7efd197ca6c,
            0x7448a8d6797a21b4,
            0x8670692cf5d4812e,
            0x0793157312883e67,
        ],
    ),
    (
        "t_mixed_4lane_through/true",
        [
            0xe07eb4072f187061,
            0x6055f486eb1a36bb,
            0x8e9b1f1be0bee144,
            0x1688825a6d3c5eca,
            0xcb77516b91670e6f,
            0x984aac3253f85f6e,
            0x047e584947353eb2,
        ],
    ),
    (
        "t_footpath_beside_arm/true",
        [
            0x9a8bc41951b5b789,
            0xd31a01050cbc1863,
            0x1f9eafee9c31ca22,
            0x5d287ad94f4132f3,
            0x00da901bb89e67be,
            0x79932d551f2bcad4,
            0x5c5a05b35e186790,
        ],
    ),
];
