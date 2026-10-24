// SPDX-License-Identifier: GPL-2.0-only

//! The roadbed product probe over the rendered reference junctions, committed stroke by stroke
//! through `SimCore::commit_road` as the reference harness does, flat and on its 2.5% slope: graph
//! profiles, node and span pieces, layouts, walkways and lanes hash exactly as recorded.

use super::road_edit_order::{ARM_M, Stroke, committed};
use crate::simulation::network::roadbed::fixtures::{ProductProbe, assert_probes};
use godot::prelude::Vector2;

// The reference harness's fixtures, by name: strokes and their commit order.
fn reference_fixtures() -> Vec<(&'static str, Vec<Stroke>, Vec<usize>)> {
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

// Recorded at `79e7a476`.
const EXPECTED: &[(&str, [u64; 7])] = &[
    (
        "t/false",
        [
            0xdcefdb84bc6a950c,
            0xd132216a076c86fe,
            0xb95e03472b0f56be,
            0xbcf712262416fd08,
            0x45ff7d0a56ec6b5f,
            0x06e550420fd5073e,
            0xd19cdbe4ea6a993e,
        ],
    ),
    (
        "four_way/false",
        [
            0xa1771f3fa157cbf1,
            0xb198b2004b87a922,
            0x4efc6f97e0bf4ca2,
            0xdbbfd84397c2ca3d,
            0x625fbff40a55bbab,
            0x5b65936359d5754e,
            0x64eb8506b7a5624d,
        ],
    ),
    (
        "y_25_branches_first/false",
        [
            0x34915670cb723d9f,
            0x30cdc5e817fb32d6,
            0xa909908d25aea486,
            0xf5bb111e5851b67d,
            0xe75bb06ba28664b4,
            0x3edb51030b26ec99,
            0xf17f78075adf78d1,
        ],
    ),
    (
        "y_25/false",
        [
            0xe2e14c4095af0a77,
            0x587495fff587e172,
            0x0e80c6a89b42a256,
            0x38ccdd2c562bd178,
            0x76c1aad9702e58a0,
            0xda68482ab9ee3afa,
            0x9ddd0d7d10716552,
        ],
    ),
    (
        "y_40/false",
        [
            0xf2e690399c7a61f5,
            0x0882351e0ad26375,
            0x986cd5044026ed8c,
            0xc354169e5b2fa337,
            0x71c63f8212054884,
            0x8506308196cefa92,
            0x67954c3a46fc2e3c,
        ],
    ),
    (
        "y_60/false",
        [
            0xaac61b1e4b2f547a,
            0xaa922d6c3db3ef99,
            0x4bbc2eb74c6692b2,
            0x9826c0b03b6f6cb8,
            0x29b2f30405e168e1,
            0xfaa38c9589858aa9,
            0xec60681378231b6d,
        ],
    ),
    (
        "bend_45/false",
        [
            0x79029c3f9f08f2d3,
            0xd281d4ef7fcb6392,
            0xdd7feff3cd848718,
            0x2bae06231f698600,
            0xa28c9f093d10c8a5,
            0xd1b6bf7ea3bcbe82,
            0x79fceb13b2eb69da,
        ],
    ),
    (
        "terminal/false",
        [
            0xfac75290dae60182,
            0x0c7ab1c3efc0beab,
            0x185656084b8d4230,
            0x4fb8dcc8341fefc0,
            0xd1dde8b070a335d2,
            0xf8a6db80088c389b,
            0x5deb7aa0b91bddd4,
        ],
    ),
    (
        "t_mixed_4lane_through/false",
        [
            0x29225eb32b5ed28a,
            0x44907748aea059e0,
            0x98b295863fce5ea4,
            0x0358774a32ac88d6,
            0x8cb271441ac4b7f0,
            0xd0f19bcdb7aa845f,
            0x4fec068359e0aa86,
        ],
    ),
    (
        "t_footpath_beside_arm/false",
        [
            0x1793286afd4aa67a,
            0x463a6de588a42db2,
            0x71d92257e0754283,
            0x8948662705fd5da4,
            0x189e1bf4ef257d5f,
            0x546233bbe2973183,
            0x826167707c189f8d,
        ],
    ),
    (
        "t/true",
        [
            0xd5b64316ed2a1887,
            0x7d56e0e2645ffb76,
            0xb95e03472b0f56be,
            0xb7c1c24b8055e516,
            0x6de43a64e8d4dfc6,
            0x953c3f55f57236e2,
            0x07c7e410d04e2803,
        ],
    ),
    (
        "four_way/true",
        [
            0x3677295d4816a142,
            0xb9217966ba6cef44,
            0x4efc6f97e0bf4ca2,
            0x3a888f1f87eb4054,
            0x23dfd267de58316b,
            0xae1ecea9bd000453,
            0xdc7a6f691016c7d8,
        ],
    ),
    (
        "y_25_branches_first/true",
        [
            0x3ed85561b8846c85,
            0x2552ed5274d6170b,
            0xa909908d25aea486,
            0x54d8d45d305ff7e3,
            0xc737d5791ec4479d,
            0x45f14468918c9787,
            0xc547286b180a4dce,
        ],
    ),
    (
        "y_25/true",
        [
            0x492c242557246ae0,
            0x3a221fc058a6aae3,
            0x0e80c6a89b42a256,
            0xa140b997d672dbd0,
            0xf547791fa21117aa,
            0x9108be4f218cee47,
            0xf75f41f8104caf5b,
        ],
    ),
    (
        "y_40/true",
        [
            0x0a045dfcf4c8351d,
            0x5b9c62b8b3d93155,
            0x986cd5044026ed8c,
            0xc4c91ca2b241db1d,
            0x40d8bee55134c620,
            0xe51687a70f56e883,
            0x0356fec34650bb8e,
        ],
    ),
    (
        "y_60/true",
        [
            0x86033475f927e538,
            0x41c63254c3ba1305,
            0x4bbc2eb74c6692b2,
            0x1fa4638ef21b7d7d,
            0x0e670fa61f673445,
            0xc9b644f57e420d78,
            0x8f6dac82158e945e,
        ],
    ),
    (
        "bend_45/true",
        [
            0x59dea7acf107049d,
            0xde32ba43aef1bf7a,
            0xdd7feff3cd848718,
            0xa0800b9699d50f6a,
            0xd0f550776251c709,
            0xc6e89ed9a82fc129,
            0x060e31015dc1a74c,
        ],
    ),
    (
        "terminal/true",
        [
            0x12b7261362dbb9c6,
            0x4678fc1a0c02a5c7,
            0x185656084b8d4230,
            0xfcaf6123e54e04ec,
            0x6f20f96a95cd0c36,
            0x5c688a5c39af7fa7,
            0x706ef28ae88e94b6,
        ],
    ),
    (
        "t_mixed_4lane_through/true",
        [
            0xe07eb4072f187061,
            0x47f2503f565acf36,
            0x98b295863fce5ea4,
            0x4f5a59643670953a,
            0xd2541ee9955b96c9,
            0x932824afadaee26a,
            0x7b09fe36804d3858,
        ],
    ),
    (
        "t_footpath_beside_arm/true",
        [
            0x9a8bc41951b5b789,
            0x5b574af27549add1,
            0x71d92257e0754283,
            0x7c35823fcd8934f3,
            0x03da16df6eaceaea,
            0x71addf488dfa4385,
            0x9a9c0de444bd76e2,
        ],
    ),
];
