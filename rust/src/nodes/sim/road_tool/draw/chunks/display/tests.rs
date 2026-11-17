// SPDX-License-Identifier: GPL-2.0-only

//! The display's collapsed runs against the recorder (`ROAD-112` round R2-4b): a preview
//! collapses exactly its runs and restores the ones it no longer hides, waits for the install
//! of the generation its runs index, shows again after an install replaced its chunk, leaves
//! the display unchanged when its runs do not fit, and holds an accepted commit's runs until
//! released.

use super::super::super::recorder::{Command, Recorder};
use super::super::ChunkRuns;
use super::*;
use crate::nodes::sim::render::stream::layers::roads::surfaces::{RoadChunkSurfaces, RoadSurface};

const MATERIALS: [Handle; LAYER_COUNT] = [11, 12, 13, 14, 15, 16, 17, 18, 19];
// Resident meshes (handles the recorder never makes) and their layer 1's surface.
const MESH_A: Handle = 1001;
const MESH_B: Handle = 1002;
const MESH_C: Handle = 1003;
const SURFACE: i32 = 2;
const VERTICES: u32 = 30;

// Vertex `index`'s published position bytes in a resident mesh: never zero.
fn published(mesh: Handle) -> Vec<u8> {
    (0..VERTICES * POSITION_BYTES)
        .map(|at| (at as u8).wrapping_add(mesh as u8) | 1)
        .collect()
}

fn resident_chunk(mesh: Handle) -> ResidentChunk {
    let mut chunk = ResidentChunk {
        mesh,
        ..ResidentChunk::default()
    };
    chunk.layers[1] = ResidentLayer {
        surface: SURFACE,
        vertices: VERTICES,
        positions_only: true,
    };
    chunk
}

fn display() -> ChunkDisplay<Recorder> {
    let mut recorder = Recorder::default();
    for mesh in [MESH_A, MESH_B, MESH_C] {
        recorder
            .resident
            .insert((mesh, SURFACE as u32), published(mesh));
    }
    ChunkDisplay::new(ChunkOverlays::new(recorder, MATERIALS))
}

// Generation 1's resident chunks: (0, 0) on mesh A, (1, 0) on mesh B.
fn resident() -> Resident {
    let mut resident = Resident::default();
    resident.set_grid(1, 256.0, (0.0, 0.0));
    resident.install((0, 0), resident_chunk(MESH_A));
    resident.install((1, 0), resident_chunk(MESH_B));
    resident
}

// A preview of generation `generation` hiding layer 1's `runs` per chunk (positions read from
// the mesh the chunk has in `resident()`), with an overlay triangle set on its first chunk.
fn preview(generation: i64, runs: &[(SurfaceChunkKey, &[(u32, u32)])]) -> Arc<PreviewOverlay> {
    let mut overlay = PreviewOverlay {
        chunks: Vec::new(),
        runs: Vec::new(),
        hidden: Vec::new(),
        positions: Vec::new(),
        generation,
        span_m: 256.0,
        origin_m: (0.0, 0.0),
    };
    for (index, &(key, chunk_runs)) in runs.iter().enumerate() {
        if index == 0 {
            let surface = RoadSurface {
                format: 0x8_0000_001f,
                vertex_count: 3,
                vertex_data: vec![7; 60],
                attribute_data: vec![7; 36],
                aabb: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            };
            let layers = vec![(1, surface)];
            overlay.chunks.push((
                key,
                RoadChunkSurfaces {
                    layers,
                    lining_shadow: None,
                },
            ));
        }
        let mesh = if key == (0, 0) { MESH_A } else { MESH_B };
        let bytes = published(mesh);
        let (from, bytes_from) = (overlay.hidden.len(), overlay.positions.len());
        for &(start, end) in chunk_runs {
            overlay.hidden.push([1, start, end]);
            let range = (start * POSITION_BYTES) as usize..(end * POSITION_BYTES) as usize;
            // A run past the mesh (a failure case) gets made-up positions.
            match bytes.get(range.clone()) {
                Some(published) => overlay.positions.extend_from_slice(published),
                None => overlay
                    .positions
                    .resize(overlay.positions.len() + range.len(), 1),
            }
        }
        overlay.runs.push(ChunkRuns {
            key,
            hidden: from..overlay.hidden.len(),
            positions: bytes_from..overlay.positions.len(),
        });
    }
    Arc::new(overlay)
}

// Mesh `mesh`'s positions as the display left them.
fn buffer(display: &ChunkDisplay<Recorder>, mesh: Handle) -> &[u8] {
    &display.pool().backend().resident[&(mesh, SURFACE as u32)]
}

// Whether vertices `start..end` of `mesh` are collapsed (zero) or published, each.
fn collapsed(display: &ChunkDisplay<Recorder>, mesh: Handle, start: u32, end: u32) -> bool {
    let range = (start * POSITION_BYTES) as usize..(end * POSITION_BYTES) as usize;
    buffer(display, mesh)[range].iter().all(|&byte| byte == 0)
}

fn writes(display: &ChunkDisplay<Recorder>) -> usize {
    (display.pool().backend().commands.iter())
        .filter(|command| matches!(command, Command::Write(..)))
        .count()
}

#[test]
fn a_preview_collapses_exactly_its_runs() {
    let mut display = display();
    let resident = resident();
    assert!(display.show(1, &preview(1, &[((0, 0), &[(3, 6), (10, 12)])]), &resident));
    assert_eq!(display.sequence(), 1);
    assert!(collapsed(&display, MESH_A, 3, 6) && collapsed(&display, MESH_A, 10, 12));
    let mut expected = published(MESH_A);
    expected[36..72].fill(0);
    expected[120..144].fill(0);
    assert_eq!(buffer(&display, MESH_A), expected.as_slice());
    assert_eq!(buffer(&display, MESH_B), published(MESH_B).as_slice());
    let state: Vec<_> = display.collapsed().collect();
    assert_eq!(state.len(), 1);
    assert_eq!(
        (state[0].key, state[0].mesh, state[0].runs),
        ((0, 0), MESH_A, 2)
    );
    assert_eq!(display.pool().state().count(), 1);
}

#[test]
fn the_next_preview_restores_what_it_no_longer_hides() {
    let mut display = display();
    let resident = resident();
    display.show(1, &preview(1, &[((0, 0), &[(3, 6), (10, 12)])]), &resident);
    let before = writes(&display);
    // (3, 6) stays, (10, 12) comes back, (0, 0)'s neighbour (1, 0) collapses.
    let next = preview(1, &[((0, 0), &[(3, 6)]), ((1, 0), &[(0, 4)])]);
    assert!(display.show(2, &next, &resident));
    assert_eq!(
        writes(&display) - before,
        2,
        "a kept run is not written again"
    );
    assert!(collapsed(&display, MESH_A, 3, 6) && !collapsed(&display, MESH_A, 10, 12));
    assert!(collapsed(&display, MESH_B, 0, 4));
    assert_eq!(
        buffer(&display, MESH_A)[120..144],
        published(MESH_A)[120..144]
    );
    // The same number again changes nothing.
    assert!(display.show(2, &next, &resident));
    assert_eq!(writes(&display) - before, 2);
    display.clear();
    assert_eq!(buffer(&display, MESH_A), published(MESH_A).as_slice());
    assert_eq!(buffer(&display, MESH_B), published(MESH_B).as_slice());
    assert_eq!((display.sequence(), display.collapsed().count()), (0, 0));
}

#[test]
fn a_run_that_moves_within_a_layer_ends_collapsed() {
    let mut display = display();
    let resident = resident();
    display.show(1, &preview(1, &[((0, 0), &[(3, 8)])]), &resident);
    display.show(2, &preview(1, &[((0, 0), &[(5, 10)])]), &resident);
    assert!(collapsed(&display, MESH_A, 5, 10));
    assert_eq!(buffer(&display, MESH_A)[36..60], published(MESH_A)[36..60]);
}

#[test]
fn a_preview_waits_for_the_install_of_its_generation() {
    let mut display = display();
    let mut resident = resident();
    display.show(1, &preview(1, &[((0, 0), &[(3, 6)])]), &resident);
    // The next stroke indexes generation 2's chunks, not installed yet.
    assert!(display.show(2, &preview(2, &[((0, 0), &[(0, 2)])]), &resident));
    assert_eq!((display.sequence(), display.pending()), (0, 2));
    assert_eq!(buffer(&display, MESH_A), published(MESH_A).as_slice());
    assert_eq!(
        display.pool().state().count(),
        0,
        "a waiting preview shows no road"
    );
    // Generation 2 installs (0, 0) on mesh C... while held, it still waits.
    resident.install((0, 0), resident_chunk(MESH_C));
    resident.set_grid(2, 256.0, (0.0, 0.0));
    resident.holding = true;
    display.resident_changed(&resident);
    assert_eq!((display.sequence(), display.pending()), (0, 2));
    // ...and shows in the call that swaps the held chunks in.
    resident.holding = false;
    display.resident_changed(&resident);
    assert_eq!((display.sequence(), display.pending()), (2, 0));
    assert!(collapsed(&display, MESH_C, 0, 2));
    assert_eq!(
        display.collapsed().next().map(|state| state.mesh),
        Some(MESH_C)
    );
}

#[test]
fn an_install_that_replaced_its_chunk_shows_it_again() {
    let mut display = display();
    let mut resident = resident();
    display.show(
        1,
        &preview(1, &[((0, 0), &[(3, 6)]), ((1, 0), &[(1, 2)])]),
        &resident,
    );
    // An install of another generation replaced (1, 0) (mesh B, still drawn: not freed) and
    // freed (0, 0)'s mesh A.
    resident.install((1, 0), resident_chunk(MESH_C));
    resident.set_grid(2, 256.0, (0.0, 0.0));
    let before = writes(&display);
    display.forget_mesh(MESH_A);
    display.resident_changed(&resident);
    // The shown preview indexes generation 1: shown again, it waits for its own chunks; the
    // old mesh B still drawn gets its run back, freed mesh A is not written.
    assert_eq!((display.sequence(), display.pending()), (0, 1));
    assert_eq!(writes(&display) - before, 1);
    assert_eq!(buffer(&display, MESH_B), published(MESH_B).as_slice());
    assert!(collapsed(&display, MESH_A, 3, 6));
    assert_eq!(display.collapsed().count(), 0);
}

#[test]
fn a_failed_stage_leaves_the_display_unchanged() {
    let mut display = display();
    let resident = resident();
    display.show(1, &preview(1, &[((0, 0), &[(3, 6)])]), &resident);
    let commands = display.pool().backend().commands.len();
    let shown: Vec<_> = display.collapsed().collect();
    let failures = [
        // Past the layer's vertices.
        preview(1, &[((0, 0), &[(28, 31)])]),
        // A chunk that is not resident.
        preview(1, &[((5, 5), &[(0, 1)])]),
        // An empty run.
        preview(1, &[((0, 0), &[(4, 4)])]),
    ];
    for failure in &failures {
        assert!(!display.show(2, failure, &resident));
    }
    // A layer without a surface, and positions that do not match the runs.
    let mut layer = (*preview(1, &[((0, 0), &[(3, 6)])])).clone_for_test();
    layer.hidden[0][0] = 4;
    assert!(!display.show(2, &Arc::new(layer), &resident));
    let mut short = (*preview(1, &[((0, 0), &[(3, 6)])])).clone_for_test();
    short.positions.pop();
    short.runs[0].positions.end -= 1;
    assert!(!display.show(2, &Arc::new(short), &resident));
    // Another grid.
    let mut grid = (*preview(1, &[((0, 0), &[(3, 6)])])).clone_for_test();
    grid.span_m = 128.0;
    assert!(!display.show(2, &Arc::new(grid), &resident));
    assert_eq!(display.pool().backend().commands.len(), commands);
    assert_eq!(display.sequence(), 1);
    assert_eq!(display.collapsed().collect::<Vec<_>>(), shown);
}

#[test]
fn a_held_commit_keeps_its_runs_until_released() {
    let mut display = display();
    let resident = resident();
    display.show(1, &preview(1, &[((0, 0), &[(3, 6)])]), &resident);
    display.hold();
    assert!(display.held() && display.sequence() == 0);
    assert!(
        collapsed(&display, MESH_A, 3, 6),
        "the held commit's runs stay collapsed"
    );
    // The next preview collapses its own runs; clearing it leaves the held ones.
    display.show(2, &preview(1, &[((1, 0), &[(2, 5)])]), &resident);
    display.clear();
    assert!(collapsed(&display, MESH_A, 3, 6) && !collapsed(&display, MESH_B, 2, 5));
    let held: Vec<_> = display
        .collapsed()
        .map(|state| (state.key, state.held))
        .collect();
    assert_eq!(held, vec![((0, 0), true)]);
    display.release_held();
    assert!(!display.held());
    assert_eq!(buffer(&display, MESH_A), published(MESH_A).as_slice());
    // A held display whose mesh was freed is released without a write.
    display.show(3, &preview(1, &[((0, 0), &[(3, 6)])]), &resident);
    display.hold();
    display.forget_mesh(MESH_A);
    let before = writes(&display);
    display.release_held();
    assert_eq!(writes(&display), before);
}

impl PreviewOverlay {
    // A copy whose runs a test edits (the overlay's surfaces are not copied).
    fn clone_for_test(&self) -> Self {
        Self {
            chunks: Vec::new(),
            runs: self.runs.clone(),
            hidden: self.hidden.clone(),
            positions: self.positions.clone(),
            generation: self.generation,
            span_m: self.span_m,
            origin_m: self.origin_m,
        }
    }
}
