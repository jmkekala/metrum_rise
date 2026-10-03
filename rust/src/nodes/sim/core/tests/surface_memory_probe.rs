// SPDX-License-Identifier: GPL-2.0-only

//! Temporary ROAD-44 heap probe; not for commit.

use super::super::populated_city::zoned_city;
use std::hint::black_box;
use std::mem::take;
use std::sync::Arc;

static mut NEXT: usize = 7_777_777;

fn phase(name: &str, work: impl FnOnce()) {
    // SAFETY: single-threaded probe.
    let size = unsafe {
        NEXT += 1013;
        NEXT
    };
    println!("PROBE_PHASE {name} {size}");
    let marker = black_box(vec![1u8; size]);
    let before = in_use_bytes();
    work();
    let after = in_use_bytes();
    drop(black_box(marker));
    println!("PROBE_DELTA {name}: {:.1} MB", (after as f64 - before as f64) / 1e6);
}

fn in_use_bytes() -> usize {
    // SAFETY: glibc query without preconditions; it sums every arena.
    let info = unsafe { mallinfo2() };
    info.uordblks + info.hblkhd
}

macro_rules! span_field {
    ($surface:expr, $name:literal, $field:ident) => {
        phase(concat!("span.", $name), || {
            for piece in $surface.compiled_visual_span_pieces.values_mut() {
                take(&mut Arc::get_mut(piece).expect("unique span piece").$field);
            }
        })
    };
}
macro_rules! node_field {
    ($surface:expr, $name:literal, $field:ident) => {
        phase(concat!("node.", $name), || {
            for piece in $surface.compiled_visual_node_pieces.values_mut() {
                take(&mut Arc::get_mut(piece).expect("unique node piece").$field);
            }
        })
    };
}

#[test]
#[ignore]
fn surface_memory_probe() {
    let side: f32 = std::env::var("PROBE_SIDE_M").map_or(2041.0, |v| v.parse().unwrap());
    let mut slot = None;
    phase("build", || slot = Some(zoned_city(side)));
    let mut core = slot.unwrap();
    println!(
        "PROBE_COUNTS spans={} nodes={} edges={}",
        core.transit_network.road_surface.compiled_visual_span_pieces.len(),
        core.transit_network.road_surface.compiled_visual_node_pieces.len(),
        core.region_graph.edge_count()
    );
    // Production publishes node pieces without compile provenance.
    for piece in core.transit_network.road_surface.compiled_visual_node_pieces.values_mut() {
        Arc::get_mut(piece).unwrap().strip_compile_provenance();
    }
    phase("core.road_mesh_chunks", || {
        take(&mut core.cached_road_mesh_chunks);
        take(&mut core.published_road_mesh_chunks);
    });
    let s = &mut core.transit_network.road_surface;
    phase("surface.chunk_caches", || {
        take(&mut s.surface_chunk_cache);
        take(&mut s.earthwork_chunk_cache);
    });
    span_field!(s, "outer_boundary_loops", outer_boundary_loops);
    span_field!(s, "terrain_clip_boundary_loops", terrain_clip_boundary_loops);
    span_field!(s, "span_owned_regions", span_owned_regions);
    phase("span.sections (shared with compiled_sections)", || {
        for piece in s.compiled_visual_span_pieces.values_mut() {
            Arc::get_mut(piece).unwrap().sections = Arc::new(Vec::new());
        }
    });
    span_field!(s, "span_earthwork_support_regions", span_earthwork_support_regions);
    span_field!(s, "earthwork_outer_boundary_loops", earthwork_outer_boundary_loops);
    span_field!(s, "render_earthwork_faces", render_earthwork_faces);
    phase("span.rest (surface_query)", || take(&mut s.compiled_visual_span_pieces).clear());
    node_field!(s, "outer_boundary_loops", outer_boundary_loops);
    node_field!(s, "terrain_clip_boundary_loops", terrain_clip_boundary_loops);
    node_field!(s, "raised_step_face_polygons", raised_step_face_polygons);
    node_field!(s, "raised_step_face_sources", raised_step_face_sources);
    node_field!(s, "explicit_vertical_step_segments", explicit_vertical_step_segments);
    node_field!(s, "node_grade_authorities", node_grade_authorities);
    node_field!(s, "node_top_surface_sources", node_top_surface_sources);
    node_field!(s, "owned_regions", owned_regions);
    node_field!(s, "boolean_debug", boolean_debug);
    node_field!(s, "earthwork_owner_sources", earthwork_owner_sources);
    node_field!(s, "earthwork_outer_boundary_loops", earthwork_outer_boundary_loops);
    node_field!(s, "render_earthwork_faces", render_earthwork_faces);
    phase("node.rest (surface_query)", || take(&mut s.compiled_visual_node_pieces).clear());
    phase("surface.compiled_sections", || take(&mut s.compiled_sections).clear());
    phase("surface.node_inputs_boundaries_topologies", || {
        take(&mut s.compiled_visual_node_inputs);
        take(&mut s.compiled_visual_node_earthwork_boundaries);
        take(&mut s.compiled_visual_node_topologies);
    });
    phase("surface.rest", || {
        core.transit_network.road_surface =
            crate::simulation::network::surface::RoadSurfaceSystem::new(16.0);
    });
    phase("lane_system", || {
        core.transit_network.lane_system = crate::simulation::network::lanes::LaneSystem::new();
    });
    phase("region_graph", || {
        core.region_graph = crate::simulation::network::graph::RegionGraph::new();
    });
    phase("allocator", || {
        core.allocator = crate::simulation::buildings::allocator::BuildingAllocator::new();
    });
    phase("zoning+rest_of_core", || drop(core));
}

#[repr(C)]
#[derive(Default)]
struct MallInfo2 {
    arena: usize,
    ordblks: usize,
    smblks: usize,
    hblks: usize,
    hblkhd: usize,
    usmblks: usize,
    fsmblks: usize,
    uordblks: usize,
    fordblks: usize,
    keepcost: usize,
}

unsafe extern "C" {
    fn mallinfo2() -> MallInfo2;
    fn malloc_trim(pad: usize) -> i32;
}

fn heap_report(label: &str) {
    // SAFETY: glibc query functions without preconditions.
    let info = unsafe { mallinfo2() };
    let rss = crate::nodes::sim::benchmark::rss_mb();
    println!(
        "RSS_PROBE {label}: in_use {:.0} MB (arena {:.0} MB, mmap {:.0} MB), rss {rss} MB",
        (info.uordblks + info.hblkhd) as f64 / 1e6,
        info.arena as f64 / 1e6,
        info.hblkhd as f64 / 1e6,
    );
}

#[test]
#[ignore]
fn rss_probe() {
    let residents: u32 = std::env::var("PROBE_RESIDENTS").map_or(100_000, |v| v.parse().unwrap());
    heap_report("start");
    let city = super::super::PopulatedCity::build(residents);
    heap_report("populated city built");
    // SAFETY: glibc trim without preconditions.
    unsafe { malloc_trim(0) };
    heap_report("after malloc_trim");
    drop(city);
}

#[test]
#[ignore]
fn polygon_vertex_probe() {
    use crate::simulation::network::surface::RoadSurfaceVisualPolygon;
    let core = zoned_city(2041.0);
    let s = &core.transit_network.road_surface;
    let stats = |name: &str, polys: &mut dyn Iterator<Item = &RoadSurfaceVisualPolygon>| {
        let (mut n, mut pts, mut tris, mut foreign, mut max_pts) = (0, 0, 0, 0, 0);
        for poly in polys {
            n += 1;
            pts += poly.points_world.len();
            max_pts = max_pts.max(poly.points_world.len());
            tris += poly.triangles_world.len();
            for tri in &poly.triangles_world {
                for v in tri {
                    if !poly.points_world.contains(v) {
                        foreign += 1;
                    }
                }
            }
        }
        println!("POLY {name}: polys={n} points={pts} max_points={max_pts} tris={tris} foreign_vertices={foreign}");
    };
    let spans = || s.compiled_visual_span_pieces.values();
    let nodes = || s.compiled_visual_node_pieces.values();
    stats("span.earthwork_faces", &mut spans().flat_map(|p| p.render_earthwork_faces.iter().map(|f| &f.polygon)));
    stats("span.outer_loops", &mut spans().flat_map(|p| &p.outer_boundary_loops));
    stats("node.owned_regions", &mut nodes().flat_map(|p| p.owned_regions.iter().map(|r| &r.polygon)));
    stats("node.raised_faces", &mut nodes().flat_map(|p| &p.raised_step_face_polygons));
    stats("node.earthwork_faces", &mut nodes().flat_map(|p| p.render_earthwork_faces.iter().map(|f| &f.polygon)));
    stats("node.outer_loops", &mut nodes().flat_map(|p| &p.outer_boundary_loops));
    let faces: usize = spans().map(|p| p.render_earthwork_faces.len()).sum();
    println!("SIZES face={} polygon={} source={} region={}", std::mem::size_of::<crate::simulation::network::surface::RoadSurfaceEarthworkRenderFace>(), std::mem::size_of::<RoadSurfaceVisualPolygon>(), std::mem::size_of::<crate::simulation::network::surface::RoadSurfaceEarthworkFaceSource>(), std::mem::size_of::<crate::simulation::network::surface::RoadSurfaceSpanOwnedRegion>());
    println!("span faces={faces}");
}
