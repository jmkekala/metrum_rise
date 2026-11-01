// SPDX-License-Identifier: GPL-2.0-only

//! Portal rows in the graded terrain (`TERRAIN-07-C`, `ROAD-61` round R8): no cone reaches past
//! a tunnel's portal row, the approach's retained seams leave the hill beside it natural (round
//! R8e), pruned equals unpruned with it, the queries answer what the bake packs, and a tunnel
//! edit selects every tile it alters.

use super::site_tests::structure_mouth;
use super::tests::{assert_exact_pruning, bake_with};
use super::tile::Evaluation;
use super::*;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::network::types::EdgeClass;

fn compiled(graph: &RegionGraph) -> RoadbedSystem {
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(graph);
    assert!(roadbed.failed_nodes().is_empty(), "fixture nodes compile");
    roadbed
}

fn tiles(half: i32) -> Vec<TileKey> {
    (-half..half)
        .flat_map(|x| (-half..half).map(move |z| TileKey { x, z }))
        .collect()
}

// test: the tunnel continuing a road level into a 1:2 hill, its portal row at x = 0. Past the row
// the graded ground is the hill itself out to the cap, over the bore and beside it; before the
// row the approach's seams are retained (round R8e), so beside the approach the ground is the
// hill too, and under the deck at the row the ground continues the hill. Pruned equals unpruned
// at every level, batch, order and worker count, and every `GL0` lattice point's packed value is
// the query's.
#[test]
fn no_cone_reaches_past_a_portal_row() {
    let (graph, terrain) = structure_mouth(EdgeClass::Tunnel, 0.5);
    let roadbed = compiled(&graph);
    let tiles = tiles(2);
    let gl0 = [TileKey { x: 0, z: 0 }, TileKey { x: 0, z: -1 }];
    assert_exact_pruning(&terrain, &roadbed, &NoSites, &tiles, &gl0, "tunnel portal");
    let surface = GradedSurface {
        terrain: &terrain,
        roadbed: &roadbed,
        sites: &NoSites,
        query_index: None,
    };
    let mut scratch = QueryScratch::default();
    let hill = |x: f64| 0.5 * (x + 20.0).max(0.0);
    // On `GL0` lattice points past the row, over the bore and beside it.
    for x in [0.5, 1.0, 5.0, 20.0, 40.0, 63.0] {
        for z in [-30.0, -6.0, -2.0, 0.0, 2.0, 6.0, 30.0] {
            let ground = surface.ground_height_at(x, z, &mut scratch);
            assert!(
                (ground - hill(x)).abs() < 1e-9,
                "({x}, {z}): ground {ground}, hill {}",
                hill(x)
            );
            assert_eq!(surface.surface_height_at(x, z, &mut scratch), ground);
        }
    }
    // Before the row the approach's seams are retained: 2 m before the row the hill stands
    // natural beside the approach, where the walls close it down to the road.
    for z in [-7.0, -4.0, 4.0, 7.0] {
        let beside = surface.ground_height_at(-2.0, z, &mut scratch);
        assert!((beside - hill(-2.0)).abs() < 1e-9, "beside the approach at z {z}: {beside}");
    }
    // Under the approach's deck at the row, where the mask discards the terrain, the ground
    // continued across the mouth is the far side's, the hill: the step down to the deck lies
    // over the approach, not in the lining's mouth.
    for x in [-1.0, -0.5] {
        let continued = surface.ground_height_at(x, 0.0, &mut scratch);
        // The rendered value: the base plus the packed `f32` delta.
        assert!(
            (continued - hill(x)).abs() < 1e-5,
            "under the deck at ({x}, 0): {continued}"
        );
    }
    let payloads = bake_with(
        &terrain,
        &roadbed,
        &NoSites,
        &tiles,
        Level::Gl0,
        Evaluation::Lanes,
    );
    for payload in &payloads {
        let origin = payload.tile.origin();
        for j in 0..payload.samples[1] {
            for i in 0..payload.samples[0] {
                let (x, z) = (
                    origin[0] + (i as f64 - 1.0) * 0.5,
                    origin[1] + (j as f64 - 1.0) * 0.5,
                );
                if !(-12.0..=12.0).contains(&x) || z.abs() > 12.0 {
                    continue;
                }
                let texel = payload.texels[j * payload.samples[0] + i];
                let base = surface.base_height_at(x, z, &mut scratch);
                let packed = base + f64::from(texel[0]);
                let ground = surface.ground_height_at(x, z, &mut scratch);
                assert_eq!(
                    packed,
                    base + f64::from((ground - base) as f32),
                    "({x}, {z})"
                );
                if x > 0.0 {
                    assert_eq!(texel[0], 0.0, "({x}, {z}) past the row");
                }
            }
        }
    }
}

// test: turning the continuation of the road into the hill from a bridge into a tunnel and
// back (its chains and wall run stay the same bits, so only the portal rows change), and from
// a road into a tunnel, selects every tile whose texels, seam row or walls change, and not all,
// at `GL0` and `GL2`.
#[test]
fn portal_edits_select_every_tile_they_alter() {
    let tiles = tiles(3);
    for (from, to) in [
        (EdgeClass::Bridge, EdgeClass::Tunnel),
        (EdgeClass::Tunnel, EdgeClass::Bridge),
        (EdgeClass::Standard, EdgeClass::Tunnel),
    ] {
        let (before_graph, terrain) = structure_mouth(from, 0.5);
        let (after_graph, _) = structure_mouth(to, 0.5);
        let before = compiled(&before_graph);
        let mut after = before.clone();
        drop(after.take_terrain_changes());
        after.mark_edge_dirty(1);
        for node in 0..3 {
            after.mark_node_dirty(node);
        }
        after.compile_dirty(&after_graph);
        assert!(after.failed_nodes().is_empty());
        let changes = after.take_terrain_changes();
        for level in [Level::Gl0, Level::Gl2] {
            let old = bake_with(
                &terrain,
                &before,
                &NoSites,
                &tiles,
                level,
                Evaluation::Lanes,
            );
            let new = bake_with(&terrain, &after, &NoSites, &tiles, level, Evaluation::Lanes);
            let mut dirty = Vec::new();
            dirty_tiles_for_road_changes(
                &terrain,
                &before,
                &after,
                &NoSites,
                &changes,
                level,
                &mut SelectScratch::default(),
                &mut dirty,
            );
            let mut changed = 0;
            for (old, new) in old.iter().zip(&new) {
                if old.texels != new.texels || old.seams != new.seams || old.walls != new.walls {
                    changed += 1;
                    assert!(
                        dirty.contains(&old.tile),
                        "{from:?} to {to:?} {level:?}: {:?}",
                        old.tile
                    );
                }
            }
            assert!(
                changed > 0 && dirty.len() < tiles.len(),
                "{from:?} to {to:?} {level:?}: {changed} changed, {} selected",
                dirty.len()
            );
        }
    }
}

// test (`ROAD-61` round R8e): a span turned into a tunnel's approach keeps its chains' bits,
// but its seams are retained, so a tile beyond the cap of the portal row whose window holds
// them gathers other inputs and is not adopted from a held bake of the bridge's.
#[test]
fn an_approach_changes_the_inputs_of_its_far_tiles() {
    let inputs = |class: EdgeClass| {
        let (graph, terrain) = structure_mouth(class, 0.5);
        let roadbed = compiled(&graph);
        let mut out = TileInputs::default();
        tile_inputs(&terrain, &roadbed, &NoSites, TileKey { x: -3, z: 0 }, Level::Gl0, &mut out)
            .expect("inputs");
        out
    };
    let (bridge, tunnel) = (inputs(EdgeClass::Bridge), inputs(EdgeClass::Tunnel));
    assert_eq!(bridge.triangles, tunnel.triangles);
    assert!(!bridge.seams.is_empty());
    assert_eq!(tunnel.seams[..bridge.seams.len()], bridge.seams[..]);
    assert!(tunnel.seams.len() > bridge.seams.len(), "the retained seams are inputs");
    assert_ne!(bridge, tunnel);
}

// The road seams of `roadbed`'s tunnel approaches (`ROAD-61` round R8e), as the bake reads them.
fn retained_seams(roadbed: &RoadbedSystem) -> Vec<super::seams::Seam> {
    let mut pieces = Vec::new();
    let mut out = Vec::new();
    let all = [-1.0e4, -1.0e4, 1.0e4, 1.0e4];
    roadbed.for_each_grounded_seam_in_bounds(all, &mut pieces, |[a, b], retained| {
        if retained {
            out.push(super::seams::Seam {
                retained,
                ..super::seams::road_seam(a, b)
            });
        }
    });
    out
}

// test (`ROAD-61` round R8e): along the approach's cut every `GL0` and `GL2` lattice point within
// the band of a retained seam names a road seam, its `sdf` that seam's exact signed distance,
// negative under the road: the exact mask discards on the seam line, which the trough's walls
// stand on. (The option T prototype hid the approach's seams there: every trough texel was
// unnamed, its `sdf` the band, and the mask cut a zigzag beside the walls.)
#[test]
fn trough_texels_name_their_seam() {
    let (graph, terrain) = structure_mouth(EdgeClass::Tunnel, 0.5);
    let roadbed = compiled(&graph);
    let retained = retained_seams(&roadbed);
    assert!(retained.len() >= 4, "the approach's chains are retained");
    let cell_m = f64::from(terrain.cell_size_m());
    for level in [Level::Gl0, Level::Gl2] {
        let (band, spacing) = (level.band_m(cell_m), level.spacing_m(cell_m));
        let payloads = bake_with(&terrain, &roadbed, &NoSites, &tiles(2), level, Evaluation::Lanes);
        let mut named = 0;
        for payload in &payloads {
            let origin = payload.tile.origin();
            for j in 0..payload.samples[1] {
                for i in 0..payload.samples[0] {
                    let (x, z) = (
                        origin[0] + (i as f64 - 1.0) * spacing,
                        origin[1] + (j as f64 - 1.0) * spacing,
                    );
                    // The cut, short of the portal row's band, where the row is named instead.
                    if !(-19.0..=-band - 0.5).contains(&x) {
                        continue;
                    }
                    let nearest = retained
                        .iter()
                        .map(|seam| seam.at(x, z).distance)
                        .fold(f64::INFINITY, f64::min);
                    if nearest >= band {
                        continue;
                    }
                    let texel = payload.texels[j * payload.samples[0] + i];
                    assert!(texel[2] >= 0.0, "{level:?} ({x}, {z}) {nearest} m from the seam");
                    assert_eq!(
                        texel[1].abs(),
                        nearest as f32,
                        "{level:?} ({x}, {z}): the sdf is the seam's distance"
                    );
                    named += 1;
                }
            }
        }
        assert!(named > 50, "{level:?}: {named} trough texels");
    }
}
