// SPDX-License-Identifier: GPL-2.0-only

//! Watertight terrain and road surface over the rendered reference junctions (`ROAD-68-D`): the
//! production patch payloads, composed from CDT tiles, regular filler and site paving, edge-match
//! across tile and patch boundaries. Every terrain edge is shared by exactly two faces with
//! bitwise equal vertices, or it borders the road, a building site or the world edge. The
//! published road chunks' surface layers edge-match the same way wherever road lies on both
//! sides of an edge.

use super::SimCore;
use super::road_edit_order::committed;
use super::road_reference_probe::reference_fixtures;
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::terrain::render_frame_centre;
use std::collections::BTreeMap;

// A rendered vertex in world space: the patch-local f32 position plus the patch centre, both
// exactly representable in f64, so equal bits mean the same rendered point.
type Key = (u64, u64, u32);
// Each composed patch's world XZ bounds `[min_x, min_z, max_x, max_z]`.
type PatchBounds = BTreeMap<(usize, usize), [f64; 4]>;

#[derive(Debug)]
struct Face {
    patch: (usize, usize),
    // Window index inside the patch, or `None` for regular filler and paving faces.
    window: Option<usize>,
    points: [(f64, f64, f32); 3],
}

fn key((x, z, y): (f64, f64, f32)) -> Key {
    (x.to_bits(), z.to_bits(), y.to_bits())
}

// Every terrain face of every composed patch payload, in world coordinates.
fn terrain_faces(core: &mut SimCore) -> (Vec<Face>, PatchBounds) {
    let inputs = core.collect_refined_terrain_patch_build_inputs(2.0);
    let patches = SimCore::build_refined_terrain_patch_cache_entries(inputs);
    let mut faces = Vec::new();
    let mut bounds = BTreeMap::new();
    for patch in &patches {
        assert_eq!(
            SimulationNode::cached_refined_cdt_failure_label(patch),
            None,
            "patch {:?} publishes",
            patch.key
        );
        let snapshot = &patch.patch;
        let centre_x = snapshot.world_origin_x + snapshot.world_size_x * 0.5;
        let centre_z = snapshot.world_origin_z + snapshot.world_size_z * 0.5;
        let key2 = (patch.key.patch_x, patch.key.patch_z);
        bounds.insert(
            key2,
            [
                f64::from(snapshot.world_origin_x),
                f64::from(snapshot.world_origin_z),
                f64::from(snapshot.world_origin_x + snapshot.world_size_x),
                f64::from(snapshot.world_origin_z + snapshot.world_size_z),
            ],
        );
        let buffers = patch.mesh_buffers.as_deref().expect("composed buffers");
        // Window faces come first, in window order; filler and paving follow.
        let mut window_ends = Vec::new();
        let mut end = 0;
        for window in &patch.windows {
            end += window
                .mesh_buffers
                .as_deref()
                .map_or(0, |window| window.terrain_indices.len() / 3);
            window_ends.push(end);
        }
        for (index, face) in buffers
            .terrain_indices
            .as_chunks::<3>()
            .0
            .iter()
            .enumerate()
        {
            let points = face.map(|vertex| {
                let p = buffers.terrain_vertices[vertex as usize];
                (
                    f64::from(p.x) + f64::from(centre_x),
                    f64::from(p.z) + f64::from(centre_z),
                    p.y,
                )
            });
            faces.push(Face {
                patch: key2,
                window: window_ends.iter().position(|&end| index < end),
                points,
            });
        }
    }
    (faces, bounds)
}

fn cross(a: (f64, f64, f32), b: (f64, f64, f32), c: (f64, f64, f32)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

// Edges no second face shares and that border no road, site or world edge, plus degenerate and
// over-shared edges, as readable lines.
fn open_terrain_edges(core: &mut SimCore) -> Vec<String> {
    let (faces, bounds) = terrain_faces(core);
    let world = bounds.values().fold(
        [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
        |[a, b, c, d], [e, f, g, h]| [a.min(*e), b.min(*f), c.max(*g), d.max(*h)],
    );
    let mut problems = Vec::new();
    let mut edges = BTreeMap::<(Key, Key), Vec<usize>>::new();
    for (index, face) in faces.iter().enumerate() {
        let [a, b, c] = face.points;
        if cross(a, b, c) == 0.0 {
            problems.push(format!("degenerate face {face:?}"));
            continue;
        }
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let (p, q) = (key(p), key(q));
            edges
                .entry(if p < q { (p, q) } else { (q, p) })
                .or_default()
                .push(index);
        }
    }
    let roadbed = &core.transit_network.roadbed;
    for ((p, q), users) in &edges {
        if users.len() == 2 {
            continue;
        }
        let face = &faces[users[0]];
        let [a, b] = [p, q].map(|k| (f64::from_bits(k.0), f64::from_bits(k.1)));
        if users.len() > 2 {
            problems.push(format!("edge {a:?}-{b:?} shared by {} faces", users.len()));
            continue;
        }
        let on_world = (a.0 == b.0 && (a.0 == world[0] || a.0 == world[2]))
            || (a.1 == b.1 && (a.1 == world[1] || a.1 == world[3]));
        if on_world {
            continue;
        }
        // Probe 5 mm outside the face across the edge.
        let third = face
            .points
            .iter()
            .find(|point| key(**point) != *p && key(**point) != *q)
            .unwrap();
        let (dx, dz) = (b.0 - a.0, b.1 - a.1);
        let length = dx.hypot(dz);
        let mut normal = (dz / length, -dx / length);
        if normal.0 * (third.0 - a.0) + normal.1 * (third.1 - a.1) > 0.0 {
            normal = (-normal.0, -normal.1);
        }
        let probe = (
            (a.0 + b.0) * 0.5 + normal.0 * 0.005,
            (a.1 + b.1) * 0.5 + normal.1 * 0.005,
        );
        if roadbed.band_at(probe.0 as f32, probe.1 as f32).is_some() {
            continue;
        }
        let site = core.allocator.building_sites.iter().any(|site| {
            site.contains_point(godot::prelude::Vector2::new(probe.0 as f32, probe.1 as f32))
        });
        if site {
            continue;
        }
        problems.push(format!(
            "open edge ({:.4}, {:.4})-({:.4}, {:.4}) length {length:.4} patch {:?} window {:?}",
            a.0, a.1, b.0, b.1, face.patch, face.window
        ));
    }
    problems
}

#[test]
fn reference_junction_terrain_is_watertight() {
    let mut failures = Vec::new();
    for sloped in [false, true] {
        for (name, strokes, order) in reference_fixtures() {
            let mut core = committed(&strokes, &order, sloped);
            let problems = open_terrain_edges(&mut core);
            if !problems.is_empty() {
                failures.push(format!(
                    "{name}/{sloped}: {} problems\n  {}",
                    problems.len(),
                    problems[..problems.len().min(12)].join("\n  ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Interior edges of the published road chunks' surface layers that do not border exactly two
// faces with bitwise equal world vertices (chunk-local f32 plus the chunk's frame centre, in f64).
// Every published road chunk face edge in world coordinates, with its faces' (layer, chunk) and
// the faces' points.
type RoadEdges = BTreeMap<(Key, Key), Vec<(usize, (i32, (i32, i32)))>>;
type FacePoints = Vec<[(f64, f64, f32); 3]>;

fn road_edges(core: &mut SimCore) -> (RoadEdges, FacePoints) {
    core.precompute_road_mesh_data();
    let roadbed = &core.transit_network.roadbed;
    let mut edges = RoadEdges::new();
    let mut thirds = Vec::new();
    let span = roadbed.chunk_span_m();
    let (origin_x, origin_z) = roadbed.chunk_origin_m();
    for (&chunk, mesh) in &core.cached_road_mesh_chunks {
        // Chunk vertices are local to the chunk's frame centre.
        let (centre_x, centre_z) = (
            f64::from(render_frame_centre(origin_x + chunk.0 as f32 * span, span)),
            f64::from(render_frame_centre(origin_z + chunk.1 as f32 * span, span)),
        );
        for (layer, vertices) in [
            &mesh.road_vertices,
            &mesh.sidewalk_vertices,
            &mesh.curb_vertices,
            &mesh.raised_step_vertices,
            &mesh.concrete_vertices,
        ]
        .into_iter()
        .enumerate()
        {
            for face in vertices.as_chunks::<3>().0 {
                let points =
                    face.map(|v| (f64::from(v.x) + centre_x, f64::from(v.z) + centre_z, v.y));
                let index = thirds.len();
                thirds.push(points);
                for (p, q) in [(0, 1), (1, 2), (2, 0)] {
                    let (p, q) = (key(points[p]), key(points[q]));
                    edges
                        .entry(if p < q { (p, q) } else { (q, p) })
                        .or_default()
                        .push((index, (layer as i32, chunk)));
                }
            }
        }
    }
    (edges, thirds)
}

fn open_road_edges(core: &mut SimCore) -> Vec<String> {
    let (edges, thirds) = road_edges(core);
    let roadbed = &core.transit_network.roadbed;
    let mut problems = Vec::new();
    for ((p, q), users) in &edges {
        if users.len() == 2 {
            continue;
        }
        let [a, b] = [p, q].map(|k| (f64::from_bits(k.0), f64::from(f32::from_bits(k.2)), f64::from_bits(k.1)));
        if users.len() > 2 {
            problems.push(format!("edge {a:?}-{b:?} shared by {} faces", users.len()));
            continue;
        }
        let (dx, dz) = (b.0 - a.0, b.2 - a.2);
        let length = dx.hypot(dz);
        // A vertical edge (curb face end) with one face is open; otherwise probe 5 mm across the
        // edge on the side away from its face: no road there is the outer seam.
        if length > 0.0 {
            let third = thirds[users[0].0]
                .iter()
                .find(|point| key(**point) != *p && key(**point) != *q)
                .unwrap();
            let mut normal = (dz / length, -dx / length);
            if normal.0 * (third.0 - a.0) + normal.1 * (third.1 - a.2) > 0.0 {
                normal = (-normal.0, -normal.1);
            }
            let probe = (
                (a.0 + b.0) * 0.5 + normal.0 * 0.005,
                (a.2 + b.2) * 0.5 + normal.1 * 0.005,
            );
            if roadbed.band_at(probe.0 as f32, probe.1 as f32).is_none() {
                continue;
            }
        }
        problems.push(format!(
            "open edge ({:.4}, {:.4}, {:.4})-({:.4}, {:.4}, {:.4}) length {length:.4} layer/chunk {:?}",
            a.0, a.1, a.2, b.0, b.1, b.2, users[0].1
        ));
    }
    problems
}

#[test]
fn reference_junction_road_chunks_are_watertight() {
    let mut failures = Vec::new();
    for sloped in [false, true] {
        for (name, strokes, order) in reference_fixtures() {
            let mut core = committed(&strokes, &order, sloped);
            let problems = open_road_edges(&mut core);
            if !problems.is_empty() {
                failures.push(format!(
                    "{name}/{sloped}: {} problems\n  {}",
                    problems.len(),
                    problems[..problems.len().min(12)].join("\n  ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Terrain seam vertices (ends of edges with one face and road across) that the road chunk over
// the same cell does not have with the same bits: the terrain patch and the road chunk over it are
// one cell of one grid in one frame, so equal world bits mean equal local bits. Open (roads.md
// question 23): the seam edges themselves are not all shared. Where a seam crosses a CDT tile line
// inside a patch, the CDT clip adds a seam vertex the road does not have; and on `y_60` a terrain
// seam edge runs straight past a road vertex 1.85 cm from its end. Both are T-vertices on straight
// seam edges; the tile-line points are the only seam vertices allowed off the road.
fn unshared_seam_vertices(core: &mut SimCore) -> Vec<String> {
    let (faces, bounds) = terrain_faces(core);
    let (road, _) = road_edges(core);
    let road_vertices: std::collections::BTreeSet<Key> =
        road.keys().flat_map(|&(p, q)| [p, q]).collect();
    let tile = crate::simulation::terrain::TERRAIN_CDT_TILE_SPAN_MM as f64 / 1000.0;
    let roadbed = &core.transit_network.roadbed;
    let (origin_x, origin_z) = roadbed.chunk_origin_m();
    let span = f64::from(roadbed.chunk_span_m());
    let mut edges = BTreeMap::<(Key, Key), Vec<usize>>::new();
    for (index, face) in faces.iter().enumerate() {
        let [a, b, c] = face.points;
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let (p, q) = (key(p), key(q));
            edges
                .entry(if p < q { (p, q) } else { (q, p) })
                .or_default()
                .push(index);
        }
    }
    let mut problems = Vec::new();
    for ((p, q), users) in &edges {
        if users.len() != 1 {
            continue;
        }
        let face = &faces[users[0]];
        let [a, b] = [p, q].map(|k| (f64::from_bits(k.0), f64::from_bits(k.1)));
        let third = face
            .points
            .iter()
            .find(|point| key(**point) != *p && key(**point) != *q)
            .unwrap();
        let (dx, dz) = (b.0 - a.0, b.1 - a.1);
        let length = dx.hypot(dz);
        let mut normal = (dz / length, -dx / length);
        if normal.0 * (third.0 - a.0) + normal.1 * (third.1 - a.1) > 0.0 {
            normal = (-normal.0, -normal.1);
        }
        let probe = (
            (a.0 + b.0) * 0.5 + normal.0 * 0.005,
            (a.1 + b.1) * 0.5 + normal.1 * 0.005,
        );
        if roadbed.band_at(probe.0 as f32, probe.1 as f32).is_none() {
            continue;
        }
        // The road chunk holding the edge's middle is the patch's cell.
        let middle = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
        let chunk = (
            ((middle.0 - f64::from(origin_x)) / span).floor() as i32,
            ((middle.1 - f64::from(origin_z)) / span).floor() as i32,
        );
        let patch = bounds[&face.patch];
        let cell = [0, 1].map(|axis| {
            let origin = f64::from([origin_x, origin_z][axis]);
            ((patch[axis] - origin) / span).round() as i32
        });
        if chunk != (cell[0], cell[1]) {
            problems.push(format!("seam edge in patch {:?} lies in road chunk {chunk:?}", face.patch));
        }
        for k in [p, q] {
            let (x, z) = (f64::from_bits(k.0), f64::from_bits(k.1));
            let on_tile_line =
                ((x - patch[0]) / tile).fract() == 0.0 || ((z - patch[1]) / tile).fract() == 0.0;
            if !road_vertices.contains(k) && !on_tile_line {
                problems.push(format!(
                    "seam vertex ({x:.4}, {z:.4}, {}) of patch {:?} is no road vertex",
                    f32::from_bits(k.2),
                    face.patch
                ));
            }
        }
    }
    problems
}

#[test]
fn reference_junction_road_and_terrain_share_seam_vertices() {
    let mut failures = Vec::new();
    for sloped in [false, true] {
        for (name, strokes, order) in reference_fixtures() {
            let mut core = committed(&strokes, &order, sloped);
            let problems = unshared_seam_vertices(&mut core);
            if !problems.is_empty() {
                failures.push(format!(
                    "{name}/{sloped}: {} problems\n  {}",
                    problems.len(),
                    problems[..problems.len().min(12)].join("\n  ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
