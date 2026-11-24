// SPDX-License-Identifier: GPL-3.0-or-later

//! Grid alignment round A2: the products of the aligned grid that round A3's deletions (the
//! list builder's clip to the patch, multi-page copies and the rules that only serve them) must
//! keep bitwise. On a legal world (2, 4 or 8 m cell, sides in 128 m steps, 512 m patches from the
//! world edge) every 64 m graded tile lies inside one render patch, so per fixture this asserts:
//! every tile a patch lists lies wholly inside that patch (no cell is clipped), every listed tile
//! is listed by exactly one patch, and the renderer's copy rule (`Graded::tile_patch_range`)
//! gives it exactly one page copy. It pins, per fixture, the patch lists' digest, the listed
//! tiles, and the digest of every listed tile's bake at each level (`GL0`-`GL2`, `GL3` at steps
//! 2, 4 and 8), the products A3 compares against.
//!
//! The pins are recordings of this round's code on the new worlds, not of a control: A2 changed
//! the worlds, so nothing older can be compared.

use super::road_edit_order::{Stroke, committed_on};
use super::*;
use crate::nodes::sim::render::stream::layers::graded::state::{Graded, Shape};
use crate::simulation::terrain::graded::tests::payload_digest;
use crate::simulation::terrain::graded::{
    Level, PatchTilesScratch, TILE_M, TileKey, bake_tiles, graded_tiles_in_rect,
};
use godot::prelude::Vector2;
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};

const LEVELS: [Level; 6] = [
    Level::Gl0,
    Level::Gl1,
    Level::Gl2,
    Level::Gl3 { step: 2 },
    Level::Gl3 { step: 4 },
    Level::Gl3 { step: 8 },
];

// Every patch's graded list, the alignment properties asserted, and the per-level bake digests,
// as one line: `lists=<digest> patches=<listing patches> tiles=<listed tiles> <level>=<digest>...`.
fn aligned_products(core: &SimCore) -> String {
    let (terrain, roadbed, sites) = (
        &core.heightmap,
        &core.transit_network.roadbed,
        &core.allocator,
    );
    // The renderer's copy rule on this world's patch grid, as `terrain.gd` sets it up.
    let origin = core.config.terrain_world_origin_m();
    let mut graded = Graded::default();
    graded.shape = Shape {
        world_min: [origin.0, origin.1],
        patch_span: terrain.chunk_span_m(),
        patch_grid: [
            terrain.render_patch_cols() as i32,
            terrain.render_patch_rows() as i32,
        ],
        block_patches: 2,
        layer_limit: 2048,
    };
    let (mut scratch, mut listed) = (PatchTilesScratch::default(), Vec::new());
    let mut owner: BTreeMap<TileKey, (usize, usize)> = BTreeMap::new();
    let mut lists = DefaultHasher::new();
    let mut patches = 0;
    for z in 0..terrain.render_patch_rows() {
        for x in 0..terrain.render_patch_cols() {
            let (a, b, c, d) = terrain.render_patch_world_bounds(x, z).unwrap();
            let rect = [a, b, c, d].map(f64::from);
            graded_tiles_in_rect(terrain, roadbed, sites, rect, &mut scratch, &mut listed).unwrap();
            if listed.is_empty() {
                continue;
            }
            patches += 1;
            (x, z, &listed).hash(&mut lists);
            for &tile in &listed {
                let [ox, oz] = tile.origin();
                assert!(
                    ox >= rect[0]
                        && oz >= rect[1]
                        && ox + TILE_M <= rect[2]
                        && oz + TILE_M <= rect[3],
                    "tile {tile:?} is clipped by patch ({x}, {z}) {rect:?}"
                );
                if let Some(first) = owner.insert(tile, (x, z)) {
                    panic!("tile {tile:?} is listed by patches {first:?} and ({x}, {z})");
                }
                let (first, last) = graded.tile_patch_range(tile);
                assert_eq!(
                    (first, last),
                    ([x as i32, z as i32], [x as i32, z as i32]),
                    "tile {tile:?} has more than one page copy"
                );
            }
        }
    }
    let tiles: Vec<TileKey> = owner.into_keys().collect();
    let mut line = format!(
        "lists={:016x} patches={patches} tiles={}",
        lists.finish(),
        tiles.len()
    );
    let mut payloads = Vec::new();
    for level in LEVELS {
        let mut baked = Vec::with_capacity(tiles.len());
        for batch in tiles.chunks(64) {
            bake_tiles(terrain, roadbed, sites, batch, level, &mut payloads).unwrap();
            baked.append(&mut payloads);
        }
        let name = match level {
            Level::Gl3 { step } => format!("gl3s{step}"),
            other => format!("{other:?}").to_lowercase(),
        };
        line += &format!(" {name}={:016x}", payload_digest(&baked));
    }
    line
}

// The default world (20,480 m, 8 m, flat) with a junction on the patch corner at the origin, a
// road along x across four patches and a diagonal through a patch corner.
fn default_world_with_roads() -> SimCore {
    let strokes: [Stroke; 4] = [
        ([Vector2::new(-1100.0, 0.0), Vector2::new(1100.0, 0.0)], 1),
        ([Vector2::new(0.0, 0.0), Vector2::new(0.0, 300.0)], 1),
        (
            [Vector2::new(400.0, -400.0), Vector2::new(640.0, -640.0)],
            2,
        ),
        (
            [Vector2::new(-512.0, -200.0), Vector2::new(-512.0, 200.0)],
            1,
        ),
    ];
    let config = WorldConfig::default();
    assert_eq!((config.width_m, config.terrain_cell_m), (20_480.0, 8.0));
    committed_on(config.width_m, &strokes, &[0, 1, 2, 3], false)
}

// The pinned products, recorded by this test on A2's tree (equal at 2 and all workers).
const DEFAULT_WORLD: &str = "lists=26557009401c858a patches=14 tiles=105 gl0=84da27259bba8e38 \
     gl1=a6a23b81391738f6 gl2=1a6c6b247fc1c638 gl3s2=187588c47115d559 gl3s4=245528a6d0ee5358 \
     gl3s8=b85a8597aacd7281";
const KUOPIO: &str = "lists=fc64a3a163274cb7 patches=24 tiles=295 gl0=16bcc2ebd70ca028 \
     gl1=b9cde5aa9fe253c1 gl2=f52bd68d88705f88 gl3s2=e0698795c831d655 gl3s4=a5f340f6a8a6d043 \
     gl3s8=068a9c130fcb046f";
const CITY_30K: &str = "lists=cb1e0a38dcc620d3 patches=70 tiles=3312 gl0=ae5f0cc3a1287cc7 \
     gl1=112887c14fcf8a21 gl2=d45358f328822ac6 gl3s2=a2ad4325822d4033 gl3s4=a278aa79ea23ef41 \
     gl3s8=f0f2c39361886d4d";

#[test]
fn default_world_tiles_lie_in_one_patch_and_keep_their_products() {
    let core = default_world_with_roads();
    assert_eq!(aligned_products(&core), DEFAULT_WORLD);
}

// The Kuopio fixture's reference save (the 37 recorded strokes on the 18,432 m, 8 m world).
#[test]
fn kuopio_tiles_lie_in_one_patch_and_keep_their_products() {
    let mut core = test_core();
    crate::nodes::sim::core::scenario::install_bootstrap_assets(&mut core);
    core.load_game_internal(super::road_terrain_plan::KUOPIO_REFERENCE_SAVE)
        .unwrap();
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    assert_eq!(aligned_products(&core), KUOPIO);
}

// The 30k benchmark city's street grid (one block wider than the floored grid, as
// `PopulatedCity::build(30_000)` builds it), parking lots, districts and every lot built (no
// households admitted, no days run: the graded products read roads and sites only), in its world
// rounded up to the 128 m step (4,736 m; it was 4,624 m). Ignored for its build time (about 30 s
// in the test profile); round A3 runs it.
#[test]
#[ignore = "30k city build; run by grid alignment round A3"]
fn city_30k_tiles_lie_in_one_patch_and_keep_their_products() {
    use crate::nodes::sim::core::populated_city::{
        crosses_for, side_m_for, zoned_city_with_crosses,
    };
    let side_m = side_m_for(30_000);
    let mut core = zoned_city_with_crosses(side_m, crosses_for(side_m) + 1);
    assert_eq!(core.config.width_m, 4_736.0);
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    assert_eq!(aligned_products(&core), CITY_30K);
}
