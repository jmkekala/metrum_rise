// SPDX-License-Identifier: GPL-2.0-only

//! The tool's drawing against the recorder (`ROAD-112` round R2-3): a draw of the same layout
//! writes in place, a new layout rebuilds, a clear hides, the bytes are the encoders', the hue
//! holds and releases, the ghost splits by verdict, and every object made is freed.

use super::recorder::{Command, Recorder};
use super::*;
use crate::nodes::sim::road_tool::frame::{GizmoHandleKind, GizmoLineClass};
use crate::nodes::sim::road_tool::hue::HueKind;

fn materials() -> DrawMaterials {
    DrawMaterials {
        line: 1001,
        depth_line: 1002,
        handle: 1003,
        hue: [1004, 1005, 1006],
        ghost_fill: [1007, 1008],
    }
}

fn draw() -> ToolDraw<Recorder> {
    let mut draw = ToolDraw::new(Recorder::default(), materials());
    draw.backend_mut().commands.clear();
    draw
}

// A stroke of `points` with its live end as a cursor handle.
fn gizmos(points: &[Vector3]) -> Gizmos {
    Gizmos {
        line_points: points.to_vec(),
        line_starts: vec![0],
        line_classes: vec![GizmoLineClass::Draft],
        handles: vec![points[0], *points.last().unwrap()],
        handle_kinds: vec![GizmoHandleKind::Endpoint, GizmoHandleKind::Cursor],
    }
}

fn line(segments: usize, offset: f32) -> Vec<Vector3> {
    (0..=segments)
        .map(|index| Vector3::new(index as f32 * 10.0, 0.0, offset))
        .collect()
}

fn adds(recorder: &Recorder) -> usize {
    (recorder.commands.iter())
        .filter(|command| matches!(command, Command::AddSurface(..)))
        .count()
}

#[test]
fn same_layout_updates_in_place() {
    let mut draw = draw();
    draw.draw_gizmos(&gizmos(&line(3, 0.0)), Vector3::ZERO);
    let [state, ..] = draw.state();
    assert!(state.visible);
    assert_eq!(state.surfaces, 2, "a line surface and a handle surface");
    assert_eq!(adds(draw.backend()), 2);
    draw.backend_mut().commands.clear();
    // The live end moves: the same segment count, so the same padded layout.
    let moved = line(3, 4.0);
    draw.draw_gizmos(&gizmos(&moved), Vector3::ONE);
    let commands = &draw.backend().commands;
    assert!(
        commands
            .iter()
            .all(|command| matches!(command, Command::Update(..) | Command::Focus(_))),
        "only in-place writes: {commands:?}"
    );
    assert_eq!(commands.iter().filter(|c| matches!(c, Command::Update(..))).count(), 4);
    // The server's copy is the encoders' bytes.
    let mut quads = LineQuadBuffers::default();
    quads.fill(&moved, &[0], |_| LineStyle::Stroke);
    let mut expected = Vec::new();
    write_vectors(&mut expected, &quads.vertices);
    let surfaces = &draw.backend().meshes[&state.mesh];
    assert_eq!(surfaces[0].vertices, expected);
    write_floats(&mut expected, &quads.attributes);
    assert_eq!(surfaces[0].attributes, expected);
    assert_eq!(surfaces[0].material, materials().line);
    assert_eq!(surfaces[1].material, materials().handle);
    assert_eq!(&surfaces[1].attributes[8..12], &3.0f32.to_le_bytes(), "the live end's kind");
    // Two triangles a quad, 16-bit: 16 quads (the minimum) of 6 indices.
    assert_eq!(surfaces[0].indices.len(), 16 * 6 * 2);
    assert_eq!(&surfaces[0].indices[12..24], &[4, 0, 5, 0, 6, 0, 6, 0, 5, 0, 7, 0]);
}

#[test]
fn a_layout_change_rebuilds() {
    let mut draw = draw();
    draw.draw_gizmos(&gizmos(&line(3, 0.0)), Vector3::ZERO);
    draw.backend_mut().commands.clear();
    // 20 segments pad to 32 quads: a new layout.
    draw.draw_gizmos(&gizmos(&line(20, 0.0)), Vector3::ZERO);
    let commands = &draw.backend().commands;
    assert!(commands.contains(&Command::Clear(draw.state()[0].mesh)));
    assert_eq!(adds(draw.backend()), 2);
    assert!(
        commands.contains(&Command::AddSurface(draw.state()[0].mesh, SurfaceKind::Lines, 128)),
        "{commands:?}"
    );
    assert!(!commands.iter().any(|command| matches!(command, Command::Update(..))));
}

#[test]
fn clear_hides() {
    let mut draw = draw();
    draw.draw_gizmos(&gizmos(&line(3, 0.0)), Vector3::ZERO);
    let mesh = draw.state()[0].mesh;
    assert!(draw.backend().shown(mesh));
    draw.clear_gizmos();
    assert!(!draw.backend().shown(mesh));
    assert!(draw.backend().meshes[&mesh].is_empty());
    assert_eq!(draw.state()[0].surfaces, 0);
    // Drawn again after a clear: rebuilt and shown.
    draw.draw_gizmos(&gizmos(&line(3, 0.0)), Vector3::ZERO);
    assert!(draw.backend().shown(mesh));
    assert_eq!(draw.state()[0].surfaces, 2);
}

// A hue of a ground stroke, in `state`.
fn hue(state: HueState, offset: f32) -> FootprintHue {
    let mut hue = FootprintHue::default();
    hue.begin();
    hue.push_stroke(&line(3, offset), HueKind::Road);
    hue.finish(4.0, |_, _| 0.0, |_, _| f64::NEG_INFINITY, state);
    hue
}

#[test]
fn the_hue_changes_state_in_place_and_holds_with_a_commit() {
    let mut draw = draw();
    let checking = hue(HueState::Checking, 0.0);
    assert!(!checking.fill.is_empty());
    draw.draw_hue(&checking);
    let current = draw.state()[1];
    assert!(current.visible);
    assert_eq!(draw.backend().meshes[&current.mesh][0].vertices, checking.fill_bytes);
    assert_eq!(draw.backend().meshes[&current.mesh][0].material, materials().hue[1]);
    draw.backend_mut().commands.clear();
    // The verdict: the same fill, the state's material, in place.
    draw.draw_hue(&hue(HueState::Valid, 0.0));
    assert!(draw.backend().commands.contains(&Command::Material(current.mesh, 0, materials().hue[0])));
    assert_eq!(adds(draw.backend()), 0);
    // The commit holds it; the next hue draws beside it until the release.
    draw.hold_hue();
    let [_, now, held, ..] = draw.state();
    assert_eq!(held, current);
    assert!(!now.visible && now.mesh != current.mesh);
    draw.draw_hue(&hue(HueState::Checking, 30.0));
    assert!(draw.backend().shown(current.mesh) && draw.backend().shown(draw.state()[1].mesh));
    draw.release_held_hue();
    assert!(!draw.backend().shown(current.mesh));
    assert!(draw.backend().shown(draw.state()[1].mesh));
    // A hidden hue hides.
    draw.draw_hue(&FootprintHue::default());
    assert!(!draw.state()[1].visible);
}

#[test]
fn the_ghost_splits_by_verdict() {
    let mut draw = draw();
    let mut ghost = Ghost::default();
    for (stroke, valid) in [true, false, true].into_iter().enumerate() {
        ghost.starts.push(ghost.ribbon.len() as u32);
        let at = stroke as f32 * 100.0;
        ghost.ribbon.extend((0..6).map(|index| Vector3::new(at, index as f32, 0.0)));
        ghost.valid.push(valid);
        for side in [-1.0, 1.0] {
            ghost.edge_starts.push(ghost.edges.len() as u32);
            ghost.edges.extend([Vector3::new(at, 0.0, side), Vector3::new(at + 50.0, 0.0, side)]);
        }
    }
    draw.draw_ghost(&ghost);
    let state = draw.state()[3];
    assert_eq!(state.surfaces, 3, "an accepted fill, a refused fill and the edges");
    let surfaces = &draw.backend().meshes[&state.mesh];
    let mut expected = Vec::new();
    let accepted: Vec<Vector3> = ghost.ribbon[0..6].iter().chain(&ghost.ribbon[12..18]).copied().collect();
    write_vectors(&mut expected, &accepted);
    assert_eq!(surfaces[0].vertices, expected);
    assert_eq!(surfaces[0].material, materials().ghost_fill[0]);
    write_vectors(&mut expected, &ghost.ribbon[6..12]);
    assert_eq!(surfaces[1].vertices, expected);
    assert_eq!(surfaces[1].material, materials().ghost_fill[1]);
    // The refused stroke's edges (strips 2 and 3, quads 2 and 3) in the refused look.
    let look = |quad: usize| f32::from_le_bytes(surfaces[2].attributes[quad * 80 + 16..quad * 80 + 20].try_into().unwrap());
    assert_eq!([look(0), look(1), look(2), look(3), look(4)], [5.0, 5.0, 6.0, 6.0, 5.0]);
    // Every stroke accepted: one fill, the edges.
    ghost.valid.fill(true);
    draw.draw_ghost(&ghost);
    assert_eq!(draw.state()[3].surfaces, 2);
    draw.clear_ghost();
    assert!(!draw.state()[3].visible);
}

#[test]
fn every_object_made_is_freed() {
    let mut draw = draw();
    draw.draw_gizmos(&gizmos(&line(3, 0.0)), Vector3::ZERO);
    draw.draw_hue(&hue(HueState::Valid, 0.0));
    draw.hold_hue();
    let live = draw.backend().live.clone();
    // Five meshes and five instances: the gizmos, two hues, the ghost, the join marks.
    assert_eq!(live.get(), 10);
    drop(draw);
    assert_eq!(live.get(), 0, "the drop frees every object the drawing made");
}

// `ROAD-118` round JS5b: the preview's join marks draw as one line surface in the hue outline's
// look (valid for the junction outline, refused for an overlapped stretch), in place while their
// padded layout holds, and nothing at all while they are unchanged (a still frame, an untouched
// pose after another).
#[test]
fn join_marks_draw_only_when_they_change() {
    let mut draw = draw();
    let square = |size: f32| {
        let mut marks = marks::JoinMarks::default();
        marks.starts.push(0);
        marks.points.extend([
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(size, 0.0, 0.0),
            Vector3::new(size, 0.0, size),
            Vector3::new(0.0, 0.0, size),
            Vector3::new(0.0, 0.0, 0.0),
        ]);
        marks
    };
    // Nothing to draw before any join: no command.
    assert!(!draw.draw_marks(&marks::JoinMarks::default()));
    assert!(draw.backend().commands.is_empty());
    let outline = square(10.0);
    assert!(draw.draw_marks(&outline));
    let mesh = draw.state()[4].mesh;
    assert!(draw.backend().shown(mesh));
    assert_eq!(
        draw.backend().commands,
        [
            Command::AddSurface(mesh, SurfaceKind::Lines, 64),
            Command::Visible(draw.state()[4].mesh + 1, true),
        ]
    );
    let surface = &draw.backend().meshes[&mesh][0];
    assert_eq!(surface.material, materials().line, "drawn over the preview, no depth test");
    let style = |surface: &recorder::RecordedSurface| {
        f32::from_le_bytes(surface.attributes[16..20].try_into().unwrap())
    };
    assert_eq!(style(surface), LineStyle::HueOutlineValid as u8 as f32);
    // Unchanged: nothing written.
    draw.backend_mut().commands.clear();
    assert!(!draw.draw_marks(&outline.clone()));
    assert!(draw.backend().commands.is_empty());
    // Moved, the same layout: written in place only.
    assert!(draw.draw_marks(&square(12.0)));
    assert!(
        (draw.backend().commands.iter()).all(|command| matches!(command, Command::Update(..))),
        "{:?}",
        draw.backend().commands
    );
    // A refusal's stretch: the refused look.
    let mut stretch = square(12.0);
    stretch.refused = true;
    assert!(draw.draw_marks(&stretch));
    assert_eq!(
        style(&draw.backend().meshes[&mesh][0]),
        LineStyle::HueOutlineRefused as u8 as f32
    );
    // The join goes: hidden once, then nothing.
    draw.backend_mut().commands.clear();
    draw.clear_marks();
    assert!(!draw.backend().shown(mesh));
    let hidden = draw.backend().commands.len();
    assert!(hidden > 0);
    assert!(!draw.draw_marks(&marks::JoinMarks::default()));
    assert_eq!(draw.backend().commands.len(), hidden);
}
