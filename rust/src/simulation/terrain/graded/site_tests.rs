// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-C` acceptance: pads at their support height bitwise, material classes against the
//! legacy paving partition, exact pruning with sites and walls, site change selection, and the
//! conflict walls closing the step at the lower road.

use super::grade::{CAP_M, TIE_IN_SLOPE};
use super::seams::Seam;
use super::sites::polygon_holds;
use super::tests::{assert_exact_pruning, bake_with};
use super::tile::{Evaluation, lattice};
use super::*;
use crate::assets::SiteSurfaceMaterial;
use crate::config::HEIGHT_SCALE;
use crate::simulation::buildings::allocator::{
    BuildingSiteSurfaceClient, SiteChange, SitePavingPartition, site_footprint_contains,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::network::roadbed::fixtures::{SLOPE, road_edge, t_junction};
use crate::simulation::network::types::NodeType;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use std::time::Instant;

/// One test site: support footprint, support height and surfaces in authored order.
#[derive(Clone, Debug)]
pub(crate) struct TestSite {
    pub(crate) footprint: Vec<Vector2>,
    pub(crate) height: f32,
    pub(crate) surfaces: Vec<(SiteSurfaceMaterial, Vec<Vector2>)>,
}

impl TestSite {
    fn bounds(&self) -> [f32; 4] {
        self.footprint
            .iter()
            .chain(self.surfaces.iter().flat_map(|(_, polygon)| polygon))
            .fold(
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
                |[a, b, c, d], p| [a.min(p.x), b.min(p.y), c.max(p.x), d.max(p.y)],
            )
    }

    /// The site's side of a change.
    pub(crate) fn change(&self) -> SiteChange {
        SiteChange {
            bounds: self.bounds(),
            support_height_m: self.height,
        }
    }
}

/// Test sites, answered by a scan of the list (test-only; the allocator uses its chunk index).
#[derive(Clone, Debug, Default)]
pub(crate) struct TestSites(pub(crate) Vec<TestSite>);

impl SiteSource for TestSites {
    fn ready(&self) -> bool {
        true
    }

    fn sites_in_bounds(&self, rect: [f64; 4], out: &mut SiteBuffer) {
        for site in &self.0 {
            let b = site.bounds().map(f64::from);
            if b[0] <= rect[2] && rect[0] <= b[2] && b[1] <= rect[3] && rect[1] <= b[3] {
                out.push_site(&site.footprint, site.height);
                for (material, polygon) in &site.surfaces {
                    out.push_surface(*material, polygon);
                }
            }
        }
    }
}

/// A rectangle of half sizes `half` centred at `centre`, turned by `degrees`, as corners.
pub(crate) fn rectangle(centre: [f32; 2], half: [f32; 2], degrees: f32) -> Vec<Vector2> {
    let (s, c) = degrees.to_radians().sin_cos();
    [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .iter()
        .map(|[u, v]| {
            let (x, z) = (u * half[0], v * half[1]);
            Vector2::new(centre[0] + c * x - s * z, centre[1] + s * x + c * z)
        })
        .collect()
}

// A 53 x 53 heightmap of 10 m cells on the sloped road plane.
fn sloped_terrain() -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(53, 53, 10.0, 53, 0.0);
    for z in 0..53 {
        for x in 0..53 {
            let wx = x as f32 * 10.0 - 260.0;
            terrain.set_height(x, z, SLOPE * wx / HEIGHT_SCALE);
        }
    }
    terrain
}

fn compiled(graph: &RegionGraph) -> RoadbedSystem {
    let mut roadbed = RoadbedSystem::new(64.0);
    roadbed.compile_dirty(graph);
    assert!(roadbed.failed_nodes().is_empty(), "fixture nodes compile");
    roadbed
}

// Three sites north of the sloped T's trunk: a turned house with an asphalt frontage apron that
// leaves its pad for the road and an overlapping concrete walk (authored order decides), a
// level pad with a non-convex concrete yard, and a pad 6 m above the road 5 m from its
// sidewalk, whose lower bound walls the road.
pub(crate) fn fixture_sites() -> TestSites {
    let turned = rectangle([14.0, -21.0], [7.0, 5.0], 12.0);
    let apron = vec![
        Vector2::new(6.5, -15.0),
        Vector2::new(13.0, -15.0),
        Vector2::new(13.0, -9.1),
        Vector2::new(6.5, -9.3),
    ];
    let walk = rectangle([12.0, -14.0], [1.0, 3.0], 0.0);
    let ell = vec![
        Vector2::new(-30.0, -28.0),
        Vector2::new(-20.0, -28.0),
        Vector2::new(-20.0, -24.0),
        Vector2::new(-26.0, -24.0),
        Vector2::new(-26.0, -14.0),
        Vector2::new(-30.0, -14.0),
    ];
    TestSites(vec![
        TestSite {
            footprint: turned,
            height: 0.65,
            surfaces: vec![
                (SiteSurfaceMaterial::Asphalt, apron),
                (SiteSurfaceMaterial::Concrete, walk),
            ],
        },
        TestSite {
            footprint: rectangle([-25.0, -22.0], [6.0, 7.0], 0.0),
            height: -0.4,
            surfaces: vec![(SiteSurfaceMaterial::Concrete, ell)],
        },
        TestSite {
            footprint: rectangle([34.0, -14.5], [4.0, 3.0], 0.0),
            height: 6.0,
            surfaces: Vec::new(),
        },
    ])
}

fn window_tiles() -> Vec<TileKey> {
    (-2..2)
        .flat_map(|x| (-2..2).map(move |z| TileKey { x, z }))
        .collect()
}

const LEVELS: [Level; 4] = [Level::Gl0, Level::Gl1, Level::Gl2, Level::Gl3 { step: 2 }];

/// The material class (0 none, 1 asphalt, 2 concrete) and edge row index of a texel's material
/// word `16 (e + 1) + class`.
pub(crate) fn material_word(texel: [f32; 4]) -> (u8, i32) {
    let word = texel[3] as i32;
    ((word % 16) as u8, word / 16 - 1)
}

fn class_of(material: Option<SiteSurfaceMaterial>) -> u8 {
    match material {
        None => 0,
        Some(SiteSurfaceMaterial::Asphalt) => 1,
        Some(SiteSurfaceMaterial::Concrete) => 2,
    }
}

/// The legacy paving partition's materials over `area` (`[min_x, min_z, max_x, max_z]`): the
/// area's two triangles partitioned by `surfaces` in their order, as `(class, triangle XZ)`.
pub(crate) fn partition_triangles(
    surfaces: &[BuildingSiteSurfaceClient],
    area: [f32; 4],
) -> Vec<(u8, [[f64; 2]; 3])> {
    let partition = SitePavingPartition::new(surfaces).expect("surfaces triangulate");
    let [x0, z0, x1, z1] = area;
    let corner = |x: f32, z: f32| Vector3::new(x, 0.0, z);
    let mut out = Vec::new();
    for triangle in [
        [corner(x0, z0), corner(x1, z0), corner(x1, z1)],
        [corner(x0, z0), corner(x1, z1), corner(x0, z1)],
    ] {
        partition.partition(triangle, |material, face| {
            out.push((
                class_of(material),
                face.map(|p| [f64::from(p.x), f64::from(p.z)]),
            ));
        });
    }
    out
}

// Distance from an XZ point to a closed triangle, 0 inside.
fn triangle_distance(t: &[[f64; 2]; 3], p: [f64; 2]) -> f64 {
    let cross =
        |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let c = [cross(t[0], t[1]), cross(t[1], t[2]), cross(t[2], t[0])];
    if c.iter().all(|&v| v >= 0.0) || c.iter().all(|&v| v <= 0.0) {
        return 0.0;
    }
    (0..3)
        .map(|i| {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dz * dz;
            let s = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / len2).clamp(0.0, 1.0);
            (p[0] - a[0] - dx * s).hypot(p[1] - a[1] - dz * s)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Asserts that every lattice point of `payloads` (at `level` over `terrain`) inside `area`
/// takes a material class of the partition triangles holding it, within the partition's f32
/// output rounding (1e-4 m), and that the class is 0 outside `area`. Returns the points checked
/// and how many were paved.
pub(crate) fn assert_classes_match_partition(
    terrain: &TerrainSystem,
    payloads: &[TilePayload],
    level: Level,
    surfaces: &[BuildingSiteSurfaceClient],
    area: [f32; 4],
) -> (usize, usize) {
    let triangles = partition_triangles(surfaces, area);
    let (mut xs, mut zs) = (Vec::new(), Vec::new());
    let (mut checked, mut paved) = (0, 0);
    for payload in payloads {
        lattice(terrain, payload.tile, level, &mut xs, &mut zs);
        for (j, &z) in zs.iter().enumerate() {
            for (i, &x) in xs.iter().enumerate() {
                let (class, _) = material_word(payload.texels[j * xs.len() + i]);
                let inside = x >= f64::from(area[0])
                    && x <= f64::from(area[2])
                    && z >= f64::from(area[1])
                    && z <= f64::from(area[3]);
                if !inside {
                    assert_eq!(class, 0, "paved outside the surfaces at ({x}, {z})");
                    continue;
                }
                let allowed: Vec<u8> = triangles
                    .iter()
                    .filter(|(_, t)| triangle_distance(t, [x, z]) <= 1e-4)
                    .map(|(class, _)| *class)
                    .collect();
                assert!(
                    allowed.contains(&class),
                    "class {class} at ({x}, {z}) of {:?}; partition {allowed:?}",
                    payload.tile
                );
                checked += 1;
                paved += usize::from(class != 0);
            }
        }
    }
    (checked, paved)
}

fn surface_clients(sites: &TestSites, order: &[usize]) -> Vec<BuildingSiteSurfaceClient> {
    order
        .iter()
        .flat_map(|&i| sites.0[i].surfaces.iter())
        .map(|(material, polygon)| BuildingSiteSurfaceClient {
            material: *material,
            name: String::new(),
            vertices_world: polygon.clone(),
        })
        .collect()
}

#[test]
fn polygon_holds_is_closed_and_exact_on_lattice_points() {
    let square = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
    let ell = [
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 2.0],
        [0.0, 2.0],
    ];
    for (x, z, inside_square, inside_ell) in [
        (1.0, 1.0, true, true),
        (0.0, 0.0, true, true),
        (2.0, 1.0, true, true),
        (1.5, 1.5, true, false),
        (1.0, 1.5, true, true),
        (2.5, 1.0, false, false),
        (-0.5, 0.5, false, false),
    ] {
        assert_eq!(
            polygon_holds(&square, x, z),
            inside_square,
            "square ({x}, {z})"
        );
        assert_eq!(polygon_holds(&ell, x, z), inside_ell, "ell ({x}, {z})");
    }
}

// Samples inside a pad equal its support height bitwise at every level; every lattice point's
// material class is the partition's; the queries report pads and paving.
#[test]
fn pads_hold_their_support_height_and_surfaces_their_partition_class() {
    let (graph, terrain, sites) = (t_junction(true), sloped_terrain(), fixture_sites());
    let roadbed = compiled(&graph);
    let tiles = window_tiles();
    let surface = GradedSurface {
        terrain: &terrain,
        roadbed: &roadbed,
        sites: &sites,
    };
    let mut scratch = QueryScratch::default();
    let (mut xs, mut zs) = (Vec::new(), Vec::new());
    for level in LEVELS {
        let payloads = bake_with(&terrain, &roadbed, &sites, &tiles, level, Evaluation::Lanes);
        let mut pad_samples = 0;
        for payload in &payloads {
            lattice(&terrain, payload.tile, level, &mut xs, &mut zs);
            for (j, &z) in zs.iter().enumerate() {
                for (i, &x) in xs.iter().enumerate() {
                    let texel = payload.texels[j * xs.len() + i];
                    let point = Vector2::new(x as f32, z as f32);
                    let Some(site) = sites
                        .0
                        .iter()
                        .find(|site| site_footprint_contains(&site.footprint, point))
                    else {
                        continue;
                    };
                    if texel[1] < 0.0 {
                        continue; // a road covers it: the road class comes first
                    }
                    let base = surface.base_height_at(x, z, &mut scratch);
                    let want = (f64::from(site.height) - base) as f32;
                    assert_eq!(
                        texel[0].to_bits(),
                        want.to_bits(),
                        "{level:?} pad sample ({x}, {z})"
                    );
                    pad_samples += 1;
                }
            }
        }
        assert!(pad_samples > 0, "{level:?} has pad samples");
        // The partition takes the surfaces in site order; the order between sites only matters
        // where their surfaces overlap, and these do not.
        let surfaces = surface_clients(&sites, &[0, 1, 2]);
        let area = sites
            .0
            .iter()
            .flat_map(|site| site.surfaces.iter().flat_map(|(_, p)| p))
            .fold(
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
                |[a, b, c, d], p| [a.min(p.x), b.min(p.y), c.max(p.x), d.max(p.y)],
            );
        let (checked, paved) =
            assert_classes_match_partition(&terrain, &payloads, level, &surfaces, area);
        // `GL3`'s 20 m lattice may miss the small surfaces altogether.
        let gl3 = matches!(level, Level::Gl3 { .. });
        assert!(
            checked > 0 && (gl3 || paved > 0),
            "{level:?}: {checked} checked, {paved} paved"
        );
        if level == Level::Gl0 {
            assert!(paved > 400, "GL0 paves the surfaces: {paved}");
        }
    }
    // The queries: a pad inside a footprint, paving on the apron outside it, the walk's overlap
    // with the apron asphalt (authored first).
    assert_eq!(surface.mask_at(14.0, -21.0, &mut scratch), GroundClass::Pad);
    assert_eq!(
        surface.mask_at(-28.0, -16.0, &mut scratch),
        GroundClass::Pad
    );
    assert_eq!(
        surface.mask_at(9.0, -10.0, &mut scratch),
        GroundClass::Paved(SiteSurfaceMaterial::Asphalt)
    );
    assert_eq!(
        surface.mask_at(12.0, -12.0, &mut scratch),
        GroundClass::Paved(SiteSurfaceMaterial::Asphalt)
    );
    let ground = surface.ground_height_at(14.0, -21.0, &mut scratch);
    assert!(
        (ground - 0.65).abs() < 1e-5,
        "the pad renders at its height: {ground}"
    );
}

// Every bake with sites stays exact: pruned equals unpruned bitwise, texels, rows, edges, walls
// and inputs, in any batch, order and worker count.
#[test]
fn site_bakes_prune_exactly() {
    let (graph, terrain, sites) = (t_junction(true), sloped_terrain(), fixture_sites());
    let roadbed = compiled(&graph);
    let gl0 = [TileKey { x: 0, z: -1 }, TileKey { x: -1, z: -1 }];
    let (conflicts, _) =
        assert_exact_pruning(&terrain, &roadbed, &sites, &window_tiles(), &gl0, "sites");
    assert!(conflicts > 0, "the high pad conflicts with the road");
}

// A placement, a removal and a height change select every tile whose bits change, and not all.
#[test]
fn site_changes_select_every_tile_they_alter() {
    let (graph, terrain, sites) = (t_junction(true), sloped_terrain(), fixture_sites());
    let roadbed = compiled(&graph);
    let tiles: Vec<TileKey> = (-3..3)
        .flat_map(|x| (-3..3).map(move |z| TileKey { x, z }))
        .collect();
    let placed = TestSite {
        footprint: rectangle([-18.0, 24.0], [5.0, 6.0], -20.0),
        height: -0.2,
        surfaces: vec![(
            SiteSurfaceMaterial::Asphalt,
            rectangle([-16.0, 15.0], [2.0, 3.0], 0.0),
        )],
    };
    let mut raised = sites.clone();
    raised.0[0].height = 1.6;
    let mut with_placed = sites.clone();
    with_placed.0.push(placed.clone());
    let mut without_high = sites.clone();
    let high = without_high.0.remove(2);
    let edits = [
        (
            "placement",
            sites.clone(),
            with_placed,
            vec![placed.change()],
        ),
        ("removal", sites.clone(), without_high, vec![high.change()]),
        (
            "height",
            sites.clone(),
            raised.clone(),
            vec![sites.0[0].change(), raised.0[0].change()],
        ),
    ];
    for (name, before, after, changes) in edits {
        for level in [Level::Gl0, Level::Gl2] {
            let old = bake_with(
                &terrain,
                &roadbed,
                &before,
                &tiles,
                level,
                Evaluation::Lanes,
            );
            let new = bake_with(&terrain, &roadbed, &after, &tiles, level, Evaluation::Lanes);
            let mut dirty = Vec::new();
            dirty_tiles_for_site_changes(
                &terrain,
                &roadbed,
                &after,
                &changes,
                level,
                &mut SelectScratch::default(),
                &mut dirty,
            );
            let mut changed = 0;
            for (old, new) in old.iter().zip(&new) {
                if old.texels != new.texels || old.edges != new.edges || old.walls != new.walls {
                    changed += 1;
                    assert!(
                        dirty.contains(&old.tile),
                        "{name} {level:?}: {:?}",
                        old.tile
                    );
                }
            }
            assert!(changed > 0, "{name} {level:?} changes tiles");
            assert!(
                dirty.len() < tiles.len(),
                "{name} {level:?} selects {}",
                dirty.len()
            );
        }
    }
}

/// The walls of `payloads` (`GL0` over `terrain`, `roadbed`, `sites`) close the step at every
/// walled seam: along every road seam in the tiles, sampled every 5 cm, where the highest lower
/// bound of the other seams stands above the seam, a face of the tile holding the point covers
/// it from the seam's height (1 mm) to at least the rendered `GL0` surface there (1 mm) and the
/// graded ground (1 cm); and each face's end is the next face's start or a wall's end. Returns
/// the walled samples and the highest wall.
pub(crate) fn assert_walls_close_the_step(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    payloads: &[TilePayload],
) -> (usize, f64) {
    let surface = GradedSurface {
        terrain,
        roadbed,
        sites,
    };
    let mut scratch = QueryScratch::default();
    let (mut walled, mut highest) = (0, 0.0f64);
    for payload in payloads {
        let origin = payload.tile.origin();
        let rect = [origin[0], origin[1], origin[0] + TILE_M, origin[1] + TILE_M];
        let (mut pieces, mut segments) = (Vec::new(), Vec::new());
        roadbed.grounded_seams_in_bounds(rect, &mut pieces, &mut segments);
        let window = [
            rect[0] - CAP_M - 2.0,
            rect[1] - CAP_M - 2.0,
            rect[2] + CAP_M + 2.0,
            rect[3] + CAP_M + 2.0,
        ];
        let mut all = Vec::new();
        roadbed.grounded_seams_in_bounds(window, &mut pieces, &mut all);
        let mut setters: Vec<Seam> = all
            .iter()
            .map(|[a, b]| super::seams::road_seam(*a, *b))
            .collect();
        let mut buffer = SiteBuffer::default();
        buffer.gather(sites, window);
        buffer.push_pad_seams(&mut setters);
        for [a, b] in &segments {
            let seam = super::seams::road_seam(*a, *b);
            let length = (seam.b[0] - seam.a[0]).hypot(seam.b[2] - seam.a[2]);
            let steps = (length / 0.05).ceil().max(1.0) as usize;
            for step in 0..=steps {
                let t = step as f64 / steps as f64;
                let (x, z) = (
                    seam.a[0] + (seam.b[0] - seam.a[0]) * t,
                    seam.a[2] + (seam.b[2] - seam.a[2]) * t,
                );
                if TileKey::at(x, z) != payload.tile {
                    continue;
                }
                let h = seam.at(x, z).height;
                let lower = setters
                    .iter()
                    .filter(|other| other.key() != seam.key())
                    // Only a seam whose cone can rise above this one at all can set `L` over
                    // it (exact: the others' lower bounds stay below the seam).
                    .filter(|other| {
                        other.height_range().1
                            - TIE_IN_SLOPE
                                * super::prune::box_distance(other.box_xz(), seam.box_xz())
                            > seam.height_range().0 - 1e-6
                    })
                    .map(|other| other.at(x, z))
                    .filter(|at| at.distance <= CAP_M)
                    .map(|at| at.height - TIE_IN_SLOPE * at.distance)
                    .fold(f64::NEG_INFINITY, f64::max);
                if lower <= h + 1e-6 {
                    continue;
                }
                let (lx, lz) = ((x - origin[0]) as f32, (z - origin[1]) as f32);
                let face = payload.walls.iter().find_map(|face| {
                    let (dx, dz) = (face.b[0] - face.a[0], face.b[1] - face.a[1]);
                    let len2 = dx * dx + dz * dz;
                    let s = if len2 > 0.0 {
                        ((lx - face.a[0]) * dx + (lz - face.a[1]) * dz) / len2
                    } else {
                        0.0
                    };
                    let off = (face.a[0] + dx * s - lx).hypot(face.a[1] + dz * s - lz);
                    ((-1e-4..=1.0 + 1e-4).contains(&s) && off < 2e-3)
                        .then_some((face, s.clamp(0.0, 1.0)))
                });
                let Some((face, s)) = face else {
                    panic!(
                        "no wall face at ({x}, {z}) of {:?}: wall {}",
                        payload.tile,
                        lower - h
                    );
                };
                let along = |pair: [f32; 2]| {
                    f64::from(pair[0]) + f64::from(pair[1] - pair[0]) * f64::from(s)
                };
                let (bottom, top) = (along(face.bottom), along(face.top));
                let rendered = surface.ground_height_at(x, z, &mut scratch);
                assert!((bottom - h).abs() < 1e-3, "bottom {bottom} at seam {h}");
                assert!(
                    top >= rendered - 1e-3,
                    "top {top} under the rendered ground {rendered} at ({x}, {z})"
                );
                assert!(
                    top >= lower - 1e-2,
                    "top {top} under the graded ground {lower} at ({x}, {z})"
                );
                walled += 1;
                highest = highest.max(lower - h);
            }
        }
        // Faces chain: each face's end starts the next face of its run.
        for pair in payload.walls.windows(2) {
            let (first, next) = (pair[0], pair[1]);
            if first.b == next.a {
                assert_eq!(first.bottom[1].to_bits(), next.bottom[0].to_bits());
                assert_eq!(first.top[1].to_bits(), next.top[0].to_bits());
            }
        }
    }
    (walled, highest)
}

// T2's decision case: two grounded roads 10 m apart (between their outer chains) with 9 m
// between their heights; the lower road's wall stands 9 - 0.5 x 10 = 4 m and closes the step.
#[test]
fn walls_close_the_step_at_the_lower_road() {
    let road = |graph: &mut RegionGraph, z: f32, y: f32| {
        let a = graph.add_node(Vector3::new(-90.0, y, z), NodeType::Junction);
        let b = graph.add_node(Vector3::new(90.0, y, z), NodeType::Junction);
        let points = (0..=9)
            .map(|i| Vector3::new(-90.0 + 20.0 * i as f32, y, z))
            .collect();
        graph.add_edge(road_edge(a, b, points, 7.0));
    };
    // The outer chain's half width, from one road alone.
    let mut single = RegionGraph::new();
    road(&mut single, 0.0, 0.0);
    single.rebuild_intersection_clips();
    let (mut pieces, mut segments) = (Vec::new(), Vec::new());
    compiled(&single).grounded_seams_in_bounds([-1e4, -1e4, 1e4, 1e4], &mut pieces, &mut segments);
    let half = segments
        .iter()
        .flat_map(|[a, b]| [a.z.abs(), b.z.abs()])
        .fold(0.0f64, f64::max) as f32;
    let mut graph = RegionGraph::new();
    road(&mut graph, 0.0, 0.0);
    road(&mut graph, 2.0 * half + 10.0, 9.0);
    graph.rebuild_intersection_clips();
    let roadbed = compiled(&graph);
    let mut terrain = TerrainSystem::with_chunking(53, 53, 10.0, 53, 0.0);
    for z in 0..53 {
        for x in 0..53 {
            terrain.set_height(x, z, 3.0 / HEIGHT_SCALE);
        }
    }
    let tiles = window_tiles();
    let payloads = bake_with(
        &terrain,
        &roadbed,
        &NoSites,
        &tiles,
        Level::Gl0,
        Evaluation::Lanes,
    );
    let (walled, highest) = assert_walls_close_the_step(&terrain, &roadbed, &NoSites, &payloads);
    assert!(
        walled > 1000,
        "the lower road is walled along its length: {walled}"
    );
    assert!(
        (highest - 4.0).abs() < 1e-3,
        "the wall stands 4 m: {highest}"
    );
    // Pruned walls equal the walls over every seam.
    let full = bake_with(
        &terrain,
        &roadbed,
        &NoSites,
        &tiles,
        Level::Gl0,
        Evaluation::Unpruned,
    );
    for (pruned, full) in payloads.iter().zip(&full) {
        assert_eq!(pruned.walls, full.walls, "{:?}", pruned.tile);
    }
}

// `TERRAIN-07-C` T22 4 (a), the graded wall test that replaces the exact cut's tunnel portal CDT
/// A road level along `x` from -90 m to a mouth at the origin, continued straight on to 90 m by
/// an edge of `class` (a pass-through's shared mouth row), over a base level to 20 m before the
/// mouth and then rising at `slope` (a 1:2 hill at 0.5, a valley at -0.5).
pub(crate) fn structure_mouth(
    class: crate::simulation::network::types::EdgeClass,
    slope: f32,
) -> (RegionGraph, TerrainSystem) {
    let mut graph = RegionGraph::new();
    let nodes = [-90.0f32, 0.0, 90.0]
        .map(|x| graph.add_node(Vector3::new(x, 0.0, 0.0), NodeType::Junction));
    let polyline = |from: f32| {
        (0..=9)
            .map(|i| Vector3::new(from + 10.0 * i as f32, 0.0, 0.0))
            .collect()
    };
    graph.add_edge(road_edge(nodes[0], nodes[1], polyline(-90.0), 7.0));
    let mut structure = road_edge(nodes[1], nodes[2], polyline(0.0), 7.0);
    structure.class = class;
    graph.add_edge(structure);
    graph.rebuild_intersection_clips();
    let mut terrain = TerrainSystem::with_chunking(53, 53, 10.0, 53, 0.0);
    for z in 0..53 {
        for x in 0..53 {
            let wx = x as f32 * 10.0 - 260.0;
            terrain.set_height(x, z, slope * (wx + 20.0).max(0.0) / HEIGHT_SCALE);
        }
    }
    (graph, terrain)
}

// test: a road running into a hill continued by a tunnel, and into a valley by a bridge, both
// level at the mouth. Along every wall seam of the structure's mouth row, every point sampled
// each 5 cm where the rendered `GL0` ground differs from the seam is covered by a face of its
// tile between the seam's height and that ground (1 mm), its lower side on the face's left: an
// abutment against the seam under the bridge; at the tunnel's portal row (T22 5 (a)) the
// terrain beyond is cut wherever the ground stands less than the clearance above the seam, and
// no face on the seam covers those points (a headwall where it stands higher). Pruned walls
// equal the walls over every seam.
#[test]
fn structure_mouths_wall_the_graded_ground() {
    use crate::simulation::network::types::EdgeClass;
    for (class, slope) in [(EdgeClass::Tunnel, 0.5f32), (EdgeClass::Bridge, -0.5)] {
        let (graph, terrain) = structure_mouth(class, slope);
        let roadbed = compiled(&graph);
        let tiles = window_tiles();
        let bake =
            |evaluation| bake_with(&terrain, &roadbed, &NoSites, &tiles, Level::Gl0, evaluation);
        let payloads = bake(Evaluation::Lanes);
        let surface = GradedSurface {
            terrain: &terrain,
            roadbed: &roadbed,
            sites: &NoSites,
        };
        let mut scratch = QueryScratch::default();
        let (mut walled, mut highest, mut opened) = (0, 0.0f64, 0);
        for payload in &payloads {
            let origin = payload.tile.origin();
            let rect = [origin[0], origin[1], origin[0] + TILE_M, origin[1] + TILE_M];
            let (mut pieces, mut segments, mut walls) = (Vec::new(), Vec::new(), Vec::new());
            roadbed.grounded_seams_in_bounds(rect, &mut pieces, &mut segments);
            roadbed.wall_seams_in_bounds(rect, &pieces, &mut walls);
            for [a, b] in &walls {
                let steps = ((b.x - a.x).hypot(b.z - a.z) / 0.05).ceil().max(1.0) as usize;
                for step in 0..=steps {
                    let t = step as f64 / steps as f64;
                    let along = |p: f64, q: f64| p + (q - p) * t;
                    let (x, z, h) = (along(a.x, b.x), along(a.z, b.z), along(a.y, b.y));
                    if TileKey::at(x, z) != payload.tile {
                        continue;
                    }
                    let ground = surface.ground_height_at(x, z, &mut scratch);
                    if (ground - h).abs() <= 1e-3 {
                        continue;
                    }
                    let (lx, lz) = ((x - origin[0]) as f32, (z - origin[1]) as f32);
                    // A face on this wall seam: both its ends on the seam's line.
                    let on_seam = |face: &&WallFace| {
                        [face.a, face.b].iter().all(|p| {
                            let (px, pz) =
                                (f64::from(p[0]) + origin[0], f64::from(p[1]) + origin[1]);
                            let (dx, dz) = (b.x - a.x, b.z - a.z);
                            ((px - a.x) * dz - (pz - a.z) * dx).abs() / dx.hypot(dz) < 2e-3
                        })
                    };
                    let covering = |face: &&WallFace| {
                        let (dx, dz) = (face.b[0] - face.a[0], face.b[1] - face.a[1]);
                        let s =
                            ((lx - face.a[0]) * dx + (lz - face.a[1]) * dz) / (dx * dx + dz * dz);
                        let off = (face.a[0] + dx * s - lx).hypot(face.a[1] + dz * s - lz);
                        (-1e-4..=1.0 + 1e-4).contains(&s) && off < 2e-3
                    };
                    if class == EdgeClass::Tunnel && ground < h + PORTAL_CLEARANCE_M {
                        assert!(
                            !payload
                                .walls
                                .iter()
                                .filter(on_seam)
                                .any(|face| covering(&face)),
                            "{class:?}: a headwall at ({x}, {z}) over the opening"
                        );
                        opened += 1;
                        continue;
                    }
                    let (face, s) = payload
                        .walls
                        .iter()
                        .find_map(|face| {
                            let (dx, dz) = (face.b[0] - face.a[0], face.b[1] - face.a[1]);
                            let s = ((lx - face.a[0]) * dx + (lz - face.a[1]) * dz)
                                / (dx * dx + dz * dz);
                            let off = (face.a[0] + dx * s - lx).hypot(face.a[1] + dz * s - lz);
                            ((-1e-4..=1.0 + 1e-4).contains(&s) && off < 2e-3)
                                .then_some((face, s.clamp(0.0, 1.0)))
                        })
                        .unwrap_or_else(|| {
                            panic!("{class:?}: no wall face at ({x}, {z}), wall {}", ground - h)
                        });
                    let lerp = |pair: [f32; 2]| f64::from(pair[0] + (pair[1] - pair[0]) * s);
                    let (bottom, top) = (lerp(face.bottom), lerp(face.top));
                    assert!(
                        (bottom - h.min(ground)).abs() < 1e-3 && (top - h.max(ground)).abs() < 1e-3,
                        "{class:?} at ({x}, {z}): face {bottom}..{top}, seam {h}, ground {ground}"
                    );
                    let forward = f64::from(face.b[0] - face.a[0]) * (b.x - a.x)
                        + f64::from(face.b[1] - face.a[1]) * (b.z - a.z)
                        > 0.0;
                    assert_eq!(forward, ground > h, "{class:?}: the lower side on the left");
                    walled += 1;
                    highest = highest.max((ground - h).abs());
                }
            }
        }
        // The rendered surface on the row interpolates the samples beside it: the hidden inside
        // rule's on the road's side, the cones of the chains' end corners beyond (up to 1.0 m
        // above the tunnel's mouth, all of it opened, 0.47 m below the bridge's).
        match class {
            EdgeClass::Tunnel => assert!(opened > 50 && walled == 0, "{opened} opened, {walled}"),
            _ => assert!(
                walled > 50 && highest > 0.25,
                "{class:?}: {walled} walled points, highest {highest}"
            ),
        }
        for (pruned, full) in payloads.iter().zip(&bake(Evaluation::Unpruned)) {
            assert_eq!(pruned.walls, full.walls, "{class:?} {:?}", pruned.tile);
        }
    }
}

// test (`TERRAIN-07-D` round 5, the `bridge_t` mouth): a road on a fill continued by a bridge
// over a valley. The hidden inside rule stands up to the capped fill arm above the road, and
// before the fix the rendered cut edge along the mouth row and the terrain beyond it rose to it
// (0.354 m over the carriageway, a dark face above the deck). Along every wall seam, sampled
// each 5 cm at every level, the rendered surface and the terrain 0.25 m beyond the row stand
// below the deck, and every face along the row is an abutment, its top the seam's height.
#[test]
fn a_bridge_mouth_over_a_valley_keeps_the_terrain_below_the_deck() {
    use crate::simulation::network::types::EdgeClass;
    let (graph, terrain) = structure_mouth(EdgeClass::Bridge, -0.5);
    let roadbed = compiled(&graph);
    let surface = GradedSurface {
        terrain: &terrain,
        roadbed: &roadbed,
        sites: &NoSites,
    };
    let mut scratch = QueryScratch::default();
    let (mut sampled, mut faces) = (0, 0);
    for level in LEVELS {
        let payloads = bake_with(
            &terrain,
            &roadbed,
            &NoSites,
            &window_tiles(),
            level,
            Evaluation::Lanes,
        );
        for payload in &payloads {
            let origin = payload.tile.origin();
            let rect = [origin[0], origin[1], origin[0] + TILE_M, origin[1] + TILE_M];
            let (mut pieces, mut segments, mut walls) = (Vec::new(), Vec::new(), Vec::new());
            roadbed.grounded_seams_in_bounds(rect, &mut pieces, &mut segments);
            roadbed.wall_seams_in_bounds(rect, &pieces, &mut walls);
            for [a, b] in &walls {
                let (dx, dz) = (b.x - a.x, b.z - a.z);
                let length = dx.hypot(dz);
                // Beyond the row: the seam's right, the road being on its left.
                let (rx, rz) = (dz / length, -dx / length);
                let steps = (length / 0.05).ceil().max(1.0) as usize;
                for step in 0..=steps {
                    let t = step as f64 / steps as f64;
                    let (x, z, h) = (a.x + dx * t, a.z + dz * t, a.y + (b.y - a.y) * t);
                    if level == Level::Gl0 && TileKey::at(x, z) == payload.tile {
                        let edge = surface.ground_height_at(x, z, &mut scratch);
                        let beyond = surface.ground_height_at(x + 0.25 * rx, z + 0.25 * rz, &mut scratch);
                        assert!(
                            edge <= h && beyond < h,
                            "at ({x}, {z}): cut edge {edge}, beyond {beyond}, deck {h}"
                        );
                        sampled += 1;
                    }
                }
                // Where a face's end lies along this wall seam, if on it.
                let along = |p: [f32; 2]| {
                    let (px, pz) = (f64::from(p[0]) + origin[0], f64::from(p[1]) + origin[1]);
                    let s = ((px - a.x) * dx + (pz - a.z) * dz) / (length * length);
                    let off = ((px - a.x) * dz - (pz - a.z) * dx).abs() / length;
                    (off < 2e-3 && (-1e-4..=1.0 + 1e-4).contains(&s)).then_some(s)
                };
                for face in payload.walls.iter() {
                    let (Some(sa), Some(sb)) = (along(face.a), along(face.b)) else {
                        continue;
                    };
                    for (s, top) in [(sa, face.top[0]), (sb, face.top[1])] {
                        let (px, pz) = (a.x + dx * s, a.z + dz * s);
                        let h = a.y + (b.y - a.y) * s.clamp(0.0, 1.0);
                        assert!(
                            (f64::from(top) - h).abs() < 1e-4,
                            "{level:?}: a face at ({px}, {pz}) tops at {top} over the deck {h}"
                        );
                    }
                    faces += 1;
                }
            }
        }
    }
    assert!(sampled > 100 && faces > 20, "{sampled} sampled, {faces} faces");
}

// The site fixture's high pad walls the road beside it, closed like a road's.
#[test]
fn a_pad_above_a_road_walls_it() {
    let (graph, terrain, sites) = (t_junction(true), sloped_terrain(), fixture_sites());
    let roadbed = compiled(&graph);
    let payloads = bake_with(
        &terrain,
        &roadbed,
        &sites,
        &window_tiles(),
        Level::Gl0,
        Evaluation::Lanes,
    );
    let (walled, highest) = assert_walls_close_the_step(&terrain, &roadbed, &sites, &payloads);
    assert!(
        walled > 50 && highest > 1.0,
        "{walled} walled, highest {highest}"
    );
}

/// The graded terrain stage of site changes (`TERRAIN-07-C`): dirty tile selection from the
/// changes, then the `GL0` bake of those tiles on the shared pool, median of 7 warm runs, as
/// JSON with the tiles and a digest of their texel bits.
pub(crate) fn site_stage(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    changes: &[SiteChange],
) -> serde_json::Value {
    let (mut tiles, mut payloads, mut scratch) = (Vec::new(), Vec::new(), SelectScratch::default());
    let mut runs = Vec::new();
    for _ in 0..7 {
        let begin = Instant::now();
        dirty_tiles_for_site_changes(
            terrain,
            roadbed,
            sites,
            changes,
            Level::Gl0,
            &mut scratch,
            &mut tiles,
        );
        let selected = begin.elapsed().as_secs_f64() * 1e3;
        bake_tiles(terrain, roadbed, sites, &tiles, Level::Gl0, &mut payloads)
            .expect("the change's tiles bake");
        runs.push((begin.elapsed().as_secs_f64() * 1e3, selected));
    }
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (stage_ms, select_ms) = runs[3];
    let bits: Vec<Vec<[u32; 4]>> = payloads
        .iter()
        .map(|p| p.texels.iter().map(|t| t.map(f32::to_bits)).collect())
        .collect();
    serde_json::json!({
        "workers": crate::thread_pools::install(rayon::current_num_threads),
        "stage_ms": stage_ms, "select_ms": select_ms, "bake_ms": stage_ms - select_ms,
        "changes": changes.len(), "tiles": tiles.len(),
        "tile_keys": tiles.iter().map(|t| (t.x, t.z)).collect::<Vec<_>>(),
        "row_edges_max": payloads.iter().map(|p| p.edges.len()).max(),
        "wall_faces": payloads.iter().map(|p| p.walls.len()).sum::<usize>(),
        "digest": format!("{:016x}", crate::debug::work::debug_hash(&bits)),
    })
}

// Sites on a 25 m grid filling the square `[-half, half]^2` outside `clear` metres of the
// origin, answered by their grid cells (no scan), as a background city of up to 100k sites.
struct GridSites {
    near: TestSites,
    half: f32,
    clear: f32,
}

const GRID_M: f32 = 25.0;

impl SiteSource for GridSites {
    fn ready(&self) -> bool {
        true
    }

    fn sites_in_bounds(&self, rect: [f64; 4], out: &mut SiteBuffer) {
        self.near.sites_in_bounds(rect, out);
        let cell = |v: f64| (v as f32 / GRID_M).floor() as i32;
        let n = (self.half / GRID_M) as i32;
        for i in cell(rect[0] - 12.0).max(-n)..=cell(rect[2] + 12.0).min(n - 1) {
            for j in cell(rect[1] - 12.0).max(-n)..=cell(rect[3] + 12.0).min(n - 1) {
                let centre = [(i as f32 + 0.5) * GRID_M, (j as f32 + 0.5) * GRID_M];
                if centre[0].abs().max(centre[1].abs()) < self.clear {
                    continue;
                }
                let footprint = rectangle(centre, [8.0, 7.0], 0.0);
                out.push_site(&footprint, 2.0 + 0.01 * i as f32);
                out.push_surface(
                    SiteSurfaceMaterial::Asphalt,
                    &rectangle([centre[0], centre[1] - 9.0], [3.0, 2.0], 0.0),
                );
            }
        }
    }
}

// `TERRAIN-07-C`: a site placement beside the sloped T, the edited neighbourhood fixed, with a
// background grid of 0 to 100k sites beyond 400 m: the same tiles with bitwise equal texels at
// every size, and the stage's cost. Set the workers with RAYON_NUM_THREADS.
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_graded_site_placement -- --ignored --nocapture"]
fn benchmark_graded_site_placement() {
    let (graph, terrain) = (t_junction(true), sloped_terrain());
    let roadbed = compiled(&graph);
    let placed = TestSite {
        footprint: rectangle([-18.0, 24.0], [5.0, 6.0], -20.0),
        height: -0.2,
        surfaces: vec![(
            SiteSurfaceMaterial::Asphalt,
            rectangle([-16.0, 15.0], [2.0, 3.0], 0.0),
        )],
    };
    let mut near = fixture_sites();
    near.0.push(placed.clone());
    let mut reference: Option<serde_json::Value> = None;
    for background in [0usize, 1_000, 10_000, 100_000] {
        // Sites per side of the grid square holding `background` sites outside the clearing.
        let half = if background == 0 {
            0.0
        } else {
            (((background as f32) * GRID_M * GRID_M + 800.0 * 800.0).sqrt() * 0.5 / GRID_M).ceil()
                * GRID_M
        };
        let sites = GridSites {
            near: near.clone(),
            half,
            clear: 400.0,
        };
        let mut record = site_stage(&terrain, &roadbed, &sites, &[placed.change()]);
        if let Some(reference) = &reference {
            assert_eq!(
                record["tile_keys"], reference["tile_keys"],
                "the same tiles"
            );
            assert_eq!(record["digest"], reference["digest"], "the same texel bits");
        } else {
            reference = Some(record.clone());
        }
        let n = (half / GRID_M) as i32;
        let count = (-n..n)
            .flat_map(|i| (-n..n).map(move |j| (i, j)))
            .filter(|&(i, j)| {
                let centre = [(i as f32 + 0.5) * GRID_M, (j as f32 + 0.5) * GRID_M];
                centre[0].abs().max(centre[1].abs()) >= 400.0
            })
            .count();
        record["background_sites"] = count.into();
        eprintln!("GRADED_SITE_PLACEMENT {record}");
    }
}

/// `TERRAIN-07-C` on an allocator's placed sites (the Kuopio yard fixture, the hillside houses):
/// for every site, the tiles within the cap of its bounds bake without errors at every level;
/// at `GL0` every uncovered lattice point in its support footprint holds its support height
/// bitwise, and every lattice point of the tiles meeting its surfaces takes the legacy
/// partition's material (the surfaces the legacy terrain snapshot hands the partition, in
/// building order). The allocator's site index must be ready. Returns the tiles baked, the pad
/// samples and the paved samples.
pub(crate) fn assert_allocator_sites_bake(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
) -> (usize, usize, usize) {
    assert!(allocator.ready(), "the site index is ready");
    let surface = GradedSurface {
        terrain,
        roadbed,
        sites: allocator,
    };
    let mut scratch = QueryScratch::default();
    let (mut baked, mut pad_samples, mut paved) = (0, 0, 0);
    let (mut xs, mut zs) = (Vec::new(), Vec::new());
    let tiles_of = |b: [f64; 4], grow: f64| {
        let (first, last) = (
            TileKey::at(b[0] - grow, b[1] - grow),
            TileKey::at(b[2] + grow, b[3] + grow),
        );
        (first.x..=last.x)
            .flat_map(|x| (first.z..=last.z).map(move |z| TileKey { x, z }))
            .collect::<Vec<_>>()
    };
    for site in &allocator.building_sites {
        let b = site.bounds();
        let bounds = [b.0, b.1, b.2, b.3].map(f64::from);
        let tiles = tiles_of(bounds, CAP_M);
        for level in LEVELS {
            let mut out = Vec::new();
            bake_tiles(terrain, roadbed, allocator, &tiles, level, &mut out).expect("sites bake");
            baked += out.len();
        }
        // Pads at GL0.
        let own = tiles_of(bounds, 1.0);
        let payloads = bake_with(
            terrain,
            roadbed,
            allocator,
            &own,
            Level::Gl0,
            Evaluation::Lanes,
        );
        for payload in &payloads {
            lattice(terrain, payload.tile, Level::Gl0, &mut xs, &mut zs);
            for (j, &z) in zs.iter().enumerate() {
                for (i, &x) in xs.iter().enumerate() {
                    let texel = payload.texels[j * xs.len() + i];
                    let point = Vector2::new(x as f32, z as f32);
                    if texel[1] < 0.0 || !site.contains_point(point) {
                        continue;
                    }
                    // The first site in canonical order holds it; sites do not overlap.
                    let base = surface.base_height_at(x, z, &mut scratch);
                    let want = (f64::from(site.support_height_m) - base) as f32;
                    assert_eq!(texel[0].to_bits(), want.to_bits(), "pad sample ({x}, {z})");
                    pad_samples += 1;
                }
            }
        }
        // Materials against the partition over the lattice of the tiles meeting the surfaces.
        let [x0, z0, x1, z1] = [
            bounds[0] - 1.0,
            bounds[1] - 1.0,
            bounds[2] + 1.0,
            bounds[3] + 1.0,
        ];
        let lattice_rect = |payload: &TilePayload| {
            let o = payload.tile.origin();
            [
                o[0] - 1.0,
                o[1] - 1.0,
                o[0] + TILE_M + 1.0,
                o[1] + TILE_M + 1.0,
            ]
            .map(|v| v as f32)
        };
        let rect = payloads.iter().map(lattice_rect).fold(
            [x0, z0, x1, z1].map(|v| v as f32),
            |[a, b, c, d], [e, f, g, h]| [a.min(e), b.min(f), c.max(g), d.max(h)],
        );
        let snapshot =
            allocator.terrain_site_snapshot_for_world_bounds(rect[0], rect[1], rect[2], rect[3]);
        let surfaces = snapshot.paving_surfaces();
        if surfaces.is_empty() {
            continue;
        }
        let area = surfaces
            .iter()
            .flat_map(|surface| &surface.vertices_world)
            .fold(
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
                |[a, b, c, d], p| [a.min(p.x), b.min(p.y), c.max(p.x), d.max(p.y)],
            );
        let (_, site_paved) =
            assert_classes_match_partition(terrain, &payloads, Level::Gl0, &surfaces, area);
        paved += site_paved;
    }
    (baked, pad_samples, paved)
}
