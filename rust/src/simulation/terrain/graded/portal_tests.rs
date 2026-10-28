// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-C` portal opening acceptance (`docs/terrain.md` T22 5 (a)): pruned equals
//! unpruned with the opening, the mask query equals the baked sign, the side walls close the
//! opening along the bore seams, and a tunnel edit selects every tile it alters.

use super::seams::segment_at;
use super::site_tests::structure_mouth;
use super::tests::{assert_exact_pruning, bake_with};
use super::tile::Evaluation;
use super::*;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{PortalPart, RoadVec3, RoadbedSystem};
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

// The roadbed's portal rows (corner segments) and bore seams meeting `rect`.
fn portal_parts(
    roadbed: &RoadbedSystem,
    rect: [f64; 4],
) -> (Vec<[RoadVec3; 2]>, Vec<[RoadVec3; 2]>) {
    let (mut rows, mut bores) = (Vec::new(), Vec::new());
    roadbed.for_each_portal_part_in_bounds(rect, &mut Vec::new(), |part| match part {
        PortalPart::Row(row) => rows.push([row[0], row[row.len() - 1]]),
        PortalPart::Bore(segment) => bores.push(segment),
        PortalPart::Top(_) => {}
    });
    (rows, bores)
}

// test: the tunnel continuing a road level into a 1:2 hill. The opening cuts the deck within
// the reach of the portal row up to where the ground stands the clearance above it, the ground
// beside it names the bore seams, and pruned equals unpruned at every level, batch, order and
// worker count. At every `GL0` lattice point around the mouth the mask query reports `Road`
// exactly where the bake's `sdf` is negative. Along the bore seams, every point sampled each
// 5 cm within the reach where the rendered ground stands below the clearance and off the deck
// edge is covered by a face of its tile between the seam's height and that ground (1 mm), the
// deck side on its left where the ground beside stands higher.
#[test]
fn a_tunnel_portal_opens_the_graded_ground() {
    let (graph, terrain) = structure_mouth(EdgeClass::Tunnel, 0.5);
    let roadbed = compiled(&graph);
    let tiles = tiles(2);
    let gl0 = [TileKey { x: 0, z: 0 }, TileKey { x: 0, z: -1 }];
    assert_exact_pruning(&terrain, &roadbed, &NoSites, &tiles, &gl0, "tunnel portal");
    let payloads = bake_with(
        &terrain,
        &roadbed,
        &NoSites,
        &tiles,
        Level::Gl0,
        Evaluation::Lanes,
    );
    let cut: u32 = payloads.iter().map(|p| p.stats.portal_cut).sum();
    let named: u32 = payloads.iter().map(|p| p.stats.bore_named).sum();
    assert!(cut > 100 && named > 20, "{cut} cut, {named} named");
    let surface = GradedSurface {
        terrain: &terrain,
        roadbed: &roadbed,
        sites: &NoSites,
        query_index: None,
    };
    let mut scratch = QueryScratch::default();
    for payload in &payloads {
        let origin = payload.tile.origin();
        for j in 0..payload.samples[1] {
            for i in 0..payload.samples[0] {
                let (x, z) = (
                    origin[0] + (i as f64 - 1.0) * 0.5,
                    origin[1] + (j as f64 - 1.0) * 0.5,
                );
                if !(-6.0..=16.0).contains(&x) || z.abs() > 10.0 {
                    continue;
                }
                let texel = payload.texels[j * payload.samples[0] + i];
                assert_eq!(
                    surface.mask_at(x, z, &mut scratch) == GroundClass::Road,
                    texel[1].is_sign_negative(),
                    "({x}, {z}) sdf {}",
                    texel[1]
                );
            }
        }
    }
    // Over the deck 5 m in, cut; 15 m in, the crown; 2.5 m beside the side 5 m in, ground.
    for (x, z, road) in [(5.0, 0.0, true), (15.0, 0.0, false), (5.0, 6.0, false)] {
        let mask = surface.mask_at(x, z, &mut scratch);
        assert_eq!(mask == GroundClass::Road, road, "({x}, {z}): {mask:?}");
    }

    let (mut walled, mut highest) = (0, 0.0f64);
    for payload in &payloads {
        let origin = payload.tile.origin();
        let rect = [origin[0], origin[1], origin[0] + TILE_M, origin[1] + TILE_M];
        let (rows, bores) = portal_parts(&roadbed, rect);
        let reaches = |x: f64, z: f64| {
            rows.iter().any(|[a, b]| {
                segment_at(a.to_array(), b.to_array(), x, z).distance <= PORTAL_REACH_M
            })
        };
        for [a, b] in &bores {
            let steps = ((b.x - a.x).hypot(b.z - a.z) / 0.05).ceil().max(1.0) as usize;
            for step in 0..=steps {
                let t = step as f64 / steps as f64;
                let along = |p: f64, q: f64| p + (q - p) * t;
                let (x, z, h) = (along(a.x, b.x), along(a.z, b.z), along(a.y, b.y));
                if TileKey::at(x, z) != payload.tile || !reaches(x, z) {
                    continue;
                }
                let ground = surface.ground_height_at(x, z, &mut scratch);
                if (ground - h).abs() <= 1e-3 || ground >= h + PORTAL_CLEARANCE_M {
                    continue;
                }
                let (lx, lz) = ((x - origin[0]) as f32, (z - origin[1]) as f32);
                let (face, s) = payload
                    .walls
                    .iter()
                    .find_map(|face| {
                        let (dx, dz) = (face.b[0] - face.a[0], face.b[1] - face.a[1]);
                        let s =
                            ((lx - face.a[0]) * dx + (lz - face.a[1]) * dz) / (dx * dx + dz * dz);
                        let off = (face.a[0] + dx * s - lx).hypot(face.a[1] + dz * s - lz);
                        ((-1e-4..=1.0 + 1e-4).contains(&s) && off < 2e-3)
                            .then_some((face, s.clamp(0.0, 1.0)))
                    })
                    .unwrap_or_else(|| panic!("no side wall at ({x}, {z}), {}", ground - h));
                let lerp = |pair: [f32; 2]| f64::from(pair[0] + (pair[1] - pair[0]) * s);
                let (bottom, top) = (lerp(face.bottom), lerp(face.top));
                assert!(
                    (bottom - h.min(ground)).abs() < 1e-3 && (top - h.max(ground)).abs() < 1e-3,
                    "({x}, {z}): face {bottom}..{top}, seam {h}, ground {ground}"
                );
                let forward = f64::from(face.b[0] - face.a[0]) * (b.x - a.x)
                    + f64::from(face.b[1] - face.a[1]) * (b.z - a.z)
                    > 0.0;
                assert_eq!(
                    forward,
                    ground > h,
                    "({x}, {z}): the lower side on the left"
                );
                walled += 1;
                highest = highest.max(ground - h);
            }
        }
    }
    assert!(
        walled > 50 && highest > 2.0,
        "{walled} side wall points, highest {highest}"
    );
}

// test: turning the continuation of the road into the hill from a bridge into a tunnel and
// back (its chains and wall run stay the same bits, so only the portal parts change), and from
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
