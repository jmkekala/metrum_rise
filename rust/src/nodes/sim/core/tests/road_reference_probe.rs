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

// Recorded at `79e7a476` (probe v2: errors by variant and values, layouts field by field) and
// re-recorded by `ROAD-68-D`: every span hash changes with the span outer chains, which now keep the
// road on their left; nodes, layouts, walkways and queries change only where an arm's knot lateral
// differs now that it divides by the segment length instead of multiplying by its reciprocal.
const EXPECTED: &[(&str, [u64; 7])] = &[
    (
        "t/false",
        [
            0xdcefdb84bc6a950c,
            0x2ad7652b52803a06,
            0x883d813476c87381,
            0x2e50ea1ca7048e48,
            0x45ff7d0a56ec6b5f,
            0x06e550420fd5073e,
            0xd19cdbe4ea6a993e,
        ],
    ),
    (
        "four_way/false",
        [
            0xa1771f3fa157cbf1,
            0xefc085504634a4e8,
            0x506e9ab4abe5e0cb,
            0xe20d31a1c3fc2d3d,
            0x625fbff40a55bbab,
            0x5b65936359d5754e,
            0x64eb8506b7a5624d,
        ],
    ),
    (
        "y_25_branches_first/false",
        [
            0x34915670cb723d9f,
            0xe9413baab1d9a574,
            0xd34b3c206dc22877,
            0x2ffa123570fbb7c3,
            0x52ce3773b9ac693d,
            0x3edb51030b26ec99,
            0xd406bbbf58d44555,
        ],
    ),
    (
        "y_25/false",
        [
            0xe2e14c4095af0a77,
            0xfccd05e2d8d24fc8,
            0x3fd1b41986e57b7f,
            0xa3be873a7e9c371e,
            0xc1bb0220f2c7a9d1,
            0xda68482ab9ee3afa,
            0x2cb89881f8477a56,
        ],
    ),
    (
        "y_40/false",
        [
            0xf2e690399c7a61f5,
            0xaf470b0d93aaf9fd,
            0x2780d8eb0d3d1c60,
            0x095403f59b0dfce3,
            0x71c63f8212054884,
            0x8506308196cefa92,
            0x67954c3a46fc2e3c,
        ],
    ),
    (
        "y_60/false",
        [
            0xaac61b1e4b2f547a,
            0xd6faadc0fac20fde,
            0xac29e9d46b88185f,
            0xb641faa35e274180,
            0x29b2f30405e168e1,
            0xfaa38c9589858aa9,
            0x9f6328808e0c2ddb,
        ],
    ),
    (
        "bend_45/false",
        [
            0x79029c3f9f08f2d3,
            0x4243509e56cdfe00,
            0xcbfb760126b89c1a,
            0x2c407fcd613cf064,
            0xa28c9f093d10c8a5,
            0xd1b6bf7ea3bcbe82,
            0x79fceb13b2eb69da,
        ],
    ),
    (
        "terminal/false",
        [
            0xfac75290dae60182,
            0x97a9834f9eb7e03b,
            0xc9b5c2d8539a655e,
            0x37aee90de597a1c0,
            0xd1dde8b070a335d2,
            0xf8a6db80088c389b,
            0x5deb7aa0b91bddd4,
        ],
    ),
    (
        "t_mixed_4lane_through/false",
        [
            0x29225eb32b5ed28a,
            0x514b9e6c44b5dc54,
            0x085afa1d9f1d1a3e,
            0x200f070ec9bdae16,
            0x8cb271441ac4b7f0,
            0xd0f19bcdb7aa845f,
            0x4fec068359e0aa86,
        ],
    ),
    (
        "t_footpath_beside_arm/false",
        [
            0x1793286afd4aa67a,
            0x192cd1c88ec3c20c,
            0xcff6ee5d3a188e7c,
            0x4c2a50ed04b807a0,
            0x189e1bf4ef257d5f,
            0x546233bbe2973183,
            0x826167707c189f8d,
        ],
    ),
    (
        "t/true",
        [
            0xd5b64316ed2a1887,
            0xb80f3e82a41f1b02,
            0x883d813476c87381,
            0x355bd2031e9a7656,
            0x6de43a64e8d4dfc6,
            0x953c3f55f57236e2,
            0x07c7e410d04e2803,
        ],
    ),
    (
        "four_way/true",
        [
            0x3677295d4816a142,
            0xf9957c2efda88e62,
            0x506e9ab4abe5e0cb,
            0xc26923b951fc8454,
            0x23dfd267de58316b,
            0xae1ecea9bd000453,
            0xdc7a6f691016c7d8,
        ],
    ),
    (
        "y_25_branches_first/true",
        [
            0x3ed85561b8846c85,
            0xce77ed41dc57581e,
            0xd34b3c206dc22877,
            0x7b7c69e2986f74ab,
            0x7f862daea3f67544,
            0x45f14468918c9787,
            0x63fe306914c5b432,
        ],
    ),
    (
        "y_25/true",
        [
            0x492c242557246ae0,
            0x815ddada4af94141,
            0x3fd1b41986e57b7f,
            0x81728f98478b09ca,
            0x913ab6b94f9656d1,
            0x9108be4f218cee47,
            0x75af90bf72b4f3ff,
        ],
    ),
    (
        "y_40/true",
        [
            0x0a045dfcf4c8351d,
            0xcd4639e79372101d,
            0x2780d8eb0d3d1c60,
            0xd105b1b7f031f669,
            0x40d8bee55134c620,
            0xe51687a70f56e883,
            0x0356fec34650bb8e,
        ],
    ),
    (
        "y_60/true",
        [
            0x86033475f927e538,
            0x915c7441ce80879e,
            0xac29e9d46b88185f,
            0x3a62cf90a55e1c5d,
            0x0e670fa61f673445,
            0xc9b644f57e420d78,
            0x1585d0e16580e6d0,
        ],
    ),
    (
        "bend_45/true",
        [
            0x59dea7acf107049d,
            0xeb2d9216e64f6ea0,
            0xcbfb760126b89c1a,
            0xb566b8e233b0e1c6,
            0xd0f550776251c709,
            0xc6e89ed9a82fc129,
            0x060e31015dc1a74c,
        ],
    ),
    (
        "terminal/true",
        [
            0x12b7261362dbb9c6,
            0xf7f14e0bcba65a5b,
            0xc9b5c2d8539a655e,
            0x7063a7efd197ca6c,
            0x6f20f96a95cd0c36,
            0x5c688a5c39af7fa7,
            0x706ef28ae88e94b6,
        ],
    ),
    (
        "t_mixed_4lane_through/true",
        [
            0xe07eb4072f187061,
            0xb2bd2ac691179bea,
            0x085afa1d9f1d1a3e,
            0xc6ce2ce7067d837a,
            0xd2541ee9955b96c9,
            0x932824afadaee26a,
            0x7b09fe36804d3858,
        ],
    ),
    (
        "t_footpath_beside_arm/true",
        [
            0x9a8bc41951b5b789,
            0xcc99da0f4831742f,
            0xcff6ee5d3a188e7c,
            0x01423f05104afbff,
            0x03da16df6eaceaea,
            0x71addf488dfa4385,
            0x9a9c0de444bd76e2,
        ],
    ),
];
