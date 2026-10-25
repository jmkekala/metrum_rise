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
const EXPECTED: &[(&str, [u64; 7])] = &[
    (
        "t/false",
        [
            0xdcefdb84bc6a950c,
            0x930a59d03d79c762,
            0x2556dcca71547b6f,
            0x47347647c0875527,
            0x4edf89ccac373d17,
            0x7a497b01f481e16a,
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
            0xe3d993390861063e,
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
            0x7907d92c4ca96ba9,
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
            0xf6c329b7c5e27393,
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
            0x26e42d2d78825d1a,
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
            0xe916cc04635a9b01,
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
            0xb36b31787d40f4ba,
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
            0xcefd5530ccacbf63,
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
            0x64cc1ab4d6f180ef,
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
            0xb5eb6a5c9f13758f,
            0x2d6c67d9c41442f0,
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
            0x57b15e41b28f604a,
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
            0x6474b67732ab4072,
            0xd095e221ec6237f7,
        ],
    ),
    (
        "y_25_branches_first/true",
        [
            0x7f558118f48130a4,
            0x453cf814acf924f2,
            0xe4c667d40f4b3c30,
            0x5c04cfcec07f9c38,
            0x23e267fd69e98af9,
            0xa5103a8c30479054,
            0x2411cf548041bb2a,
        ],
    ),
    (
        "y_25/true",
        [
            0x48f69d93dc864e28,
            0x114b3d86330fa396,
            0x2658fc21c04f8df4,
            0xf76f10dd4d7cc118,
            0xa3871c95fed3ed6d,
            0x146f19541c5c34cd,
            0x3bf1995fc2861986,
        ],
    ),
    (
        "y_40/true",
        [
            0x2eb45ace554a97c6,
            0x46c3d2f3887e4d87,
            0x282bbc68e8e7ad2b,
            0x413ffbf5036d7f92,
            0x99774b91ae59818f,
            0x9b339dbcb6f252d3,
            0xb8798c4e87523b63,
        ],
    ),
    (
        "y_60/true",
        [
            0xa5c04c8a83634f15,
            0x208597362724688a,
            0x2108b450ccc98488,
            0xbf244973b15d4002,
            0x10b00f40d981054c,
            0xbe73b019b55cf23f,
            0x2962e28f0ac2fe96,
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
            0x61f0df47c0dd65e2,
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
            0x73b20e748e871467,
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
            0xf9f521f92366dae0,
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
            0xb714adce43010eaf,
            0x288767f5cc3d19f9,
        ],
    ),
];
