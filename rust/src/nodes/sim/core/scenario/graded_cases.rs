// SPDX-License-Identifier: GPL-2.0-only

//! The graded terrain over a grown town (`TERRAIN-07-D`, `T7`): every tile the town's patches
//! list bakes at every level and `GL3` step, so no residency request fails by contract on a
//! populated world. The graded fixtures carry roads only or a few sites; here the sites and
//! their surfaces are the bootstrap packs' houses, shops and yards, as the game grows them.

use super::census_cases::busy_town;
use crate::simulation::terrain::graded::{
    Level, PatchTilesScratch, bake_tiles, graded_tiles_in_rect,
};

// Every level Godot requests a tile at.
const LEVELS: [Level; 6] = [
    Level::Gl0,
    Level::Gl1,
    Level::Gl2,
    Level::Gl3 { step: 2 },
    Level::Gl3 { step: 4 },
    Level::Gl3 { step: 8 },
];

// Round 5's populated save never settled: `GL2`'s 64 material edges held less than its dense
// tiles name (up to 119), so those bakes failed by contract. The town's rows must also stay
// within half of each capacity, `T7`'s sizing rule (twice the measured maximum): the town names
// up to 57 edges at `GL2`, which the old capacity failed by that rule before a city overflowed it.
#[test]
fn every_listed_tile_of_a_grown_town_bakes_at_every_level() {
    let mut scenario = busy_town();
    let core = &mut scenario.core;
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let (terrain, roadbed, sites) = (
        &core.heightmap,
        &core.transit_network.roadbed,
        &core.allocator,
    );
    let (mut scratch, mut listed, mut tiles) =
        (PatchTilesScratch::default(), Vec::new(), Vec::new());
    for z in 0..terrain.render_patch_rows() {
        for x in 0..terrain.render_patch_cols() {
            let (a, b, c, d) = terrain.render_patch_world_bounds(x, z).unwrap();
            let rect = [a, b, c, d].map(f64::from);
            graded_tiles_in_rect(terrain, roadbed, sites, rect, &mut scratch, &mut listed)
                .unwrap();
            tiles.extend_from_slice(&listed);
        }
    }
    tiles.sort();
    tiles.dedup();
    let mut payloads = Vec::new();
    for level in LEVELS {
        let baked = bake_tiles(terrain, roadbed, sites, &tiles, level, &mut payloads);
        assert_eq!(baked, Ok(()), "{level:?} over {} listed tiles", tiles.len());
        let seams = payloads.iter().map(|payload| payload.seams.len()).max();
        let edges = payloads.iter().map(|payload| payload.edges.len()).max();
        let (seams, edges) = (seams.unwrap(), edges.unwrap());
        // The town is populated: its yards name material edges at every level.
        assert!(edges > 0, "{level:?}: no material edge over {} tiles", tiles.len());
        assert!(
            2 * seams <= level.seam_capacity() && 2 * edges <= level.edge_capacity(),
            "{level:?}: rows of {seams} seams and {edges} edges against capacities {} and {}",
            level.seam_capacity(),
            level.edge_capacity()
        );
    }
}
