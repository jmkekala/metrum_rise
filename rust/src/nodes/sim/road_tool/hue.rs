// SPDX-License-Identifier: GPL-2.0-only

//! The draft's footprint hue (`ROAD-112` round HU): a translucent mark of the ground a draft's
//! road takes (carriageway plus sidewalks), drawn while dragging so the draft reads as a preview
//! where the preview terrain's grass still covers the preview road (T27 (d)). A **stopgap**: it
//! marks the footprint and does not fix the lag (`TERRAIN-14`).
//!
//! The hue is one non-overlapping mesh over the draft's whole footprint: the draft's strokes
//! (their prepared points, in drawing order) joined into one path and widened by the road's half
//! width with mitred joints, so its alpha never stacks where the preview road's pieces overlap
//! (junctions, a stroke's joints). It is drawn with the normal depth test only (the user,
//! 2026-11-14: it must not show through hills, trees or buildings), so it lies on whatever is
//! drawn on top: each vertex at the higher of the road and the ground there, [`HUE_LIFT_M`] up
//! (over a buried road, on the grass above it; over a fill, on the road). The ground is the one
//! the segmentation reads (`DraftGround`: the highest grounded carriageway, else the source
//! terrain). Each segment takes its stroke's kind ([`HueKind`]): a ground or cut road filled; a
//! bridge filled on the ground under the deck (the deck is not read); a tunnel only outlined (a
//! closed strip round each tunnel run, lifted the same way), never filled. The fill is cut into
//! cells of at most [`GROUND_STEP_M`] along and [`GROUND_STEP_ACROSS_M`] across, so it follows
//! the ground between the road's points. Its state ([`HueState`]) follows the draft's preview.
//!
//! Round HU2: where the preview draws road pieces, a ground or cut stroke's fill is those pieces
//! themselves ([`surface`]): their top triangles and curb faces (carriageway, kerbs, sidewalks,
//! junction corners, end caps), each point at least the lift above both the piece and the drawn
//! ground the caller reads there. Round HU3 (decision HU2-1 (a)): the session reads the drawn
//! graded surface ([`graded_ground`], through the view's shared query index), so the hue stays
//! above existing roads' cuts and fills too. The path's widened strip above stays the fill of
//! a draft the preview draws no pieces for (a refused or pending pose) and of bridges.
//!
//! Rust builds the mesh and decides its state and when it shows; `road_tool_hue.gd` draws it.
//! Cost per pose: `O(path points + footprint area / 8 m²)` vertices, one ground query each
//! (`O(pieces in its query cell)`); the buffers are reused, so a pose allocates nothing once
//! they have grown.

use super::batch::{plan_normal, ribbon_offset};
use super::frame::{Phase, RoadToolFrame};
use super::height::apply_grade_line;
use crate::simulation::network::roadbed::PieceMesh;
use crate::simulation::network::road_edit::RoadProfile;
use crate::simulation::network::types::EdgeClass;
use godot::prelude::Vector3;

/// How far the hue lies above the higher of the road and the ground, metres: clear of the
/// surface it marks without floating visibly (the shader also pulls it a little towards the
/// camera, so the rendered terrain's own triangles and levels do not cover it).
pub(crate) const HUE_LIFT_M: f32 = 0.05;
/// The largest cell of the hue along the road, metres: the terrain's 2 m cells.
pub(crate) const GROUND_STEP_M: f32 = 2.0;
/// The largest cell of the hue across the road, metres (a road's width rarely bends the ground).
pub(crate) const GROUND_STEP_ACROSS_M: f32 = 4.0;
// The fill holds at least this many triangles, padded to a power of two with collapsed ones, so
// a drag keeps the surface's layout and uploads in place.
const FILL_TRIANGLES_MIN: usize = 64;

/// The state the hue shows, from the round V palette: the draft's preview accepts it, is still
/// checking it (a batch whose tier 2 runs, a pending preview), or refuses it (drawn hatched as
/// well as red, so it does not rest on colour alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HueState {
    /// The preview accepts the draft.
    Valid = 0,
    /// The preview still checks the draft.
    Checking = 1,
    /// The preview refuses the draft.
    Refused = 2,
}

impl HueState {
    /// The state of a preview's verdict, read as the preview road's own placement tint reads it:
    /// pending first, then valid.
    pub(crate) fn of_preview(valid: bool, pending: bool) -> Self {
        match (pending, valid) {
            (true, _) => Self::Checking,
            (false, true) => Self::Valid,
            (false, false) => Self::Refused,
        }
    }
}

/// How a stroke's stretch of the footprint is marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HueKind {
    /// A ground road or a cut: filled on the road.
    Road,
    /// A bridge: filled on the ground under the deck.
    Bridge,
    /// A tunnel: outlined at the road, not filled.
    Tunnel,
}

impl HueKind {
    /// The kind of a stroke of `profile` whose preparation made it `class` (a follow-ground
    /// stroke can be an inferred bridge).
    pub(crate) fn of(profile: RoadProfile, class: EdgeClass) -> Self {
        match (profile, class) {
            (RoadProfile::Tunnel, _) | (_, EdgeClass::Tunnel) => Self::Tunnel,
            (RoadProfile::Bridge, _) | (_, EdgeClass::Bridge) => Self::Bridge,
            _ => Self::Road,
        }
    }
}

/// The draft's footprint hue: its mesh, its state and a revision that changes whenever either
/// does (the frame sends the revision; the mesh is fetched only when it changed).
#[derive(Debug, Default)]
pub(crate) struct FootprintHue {
    // The draft's path (its strokes joined) and each segment's kind; reused.
    path: Vec<Vector3>,
    kinds: Vec<HueKind>,
    // The filled triangles before the padding.
    fill_used: usize,
    /// The fill: a triangle list, padded with collapsed triangles (at the first vertex) to a
    /// power-of-two count of at least 64 (none without a fill).
    pub(crate) fill: Vec<Vector3>,
    /// [`Self::fill`] as the vertex buffer's bytes (three little-endian `f32` a vertex), what
    /// the drawing side uploads as it is.
    pub(crate) fill_bytes: Vec<u8>,
    /// The tunnel runs' outlines, closed strips back to back.
    pub(crate) outline: Vec<Vector3>,
    /// Each outline strip's first point in [`Self::outline`].
    pub(crate) outline_starts: Vec<u32>,
    // The draft's drawn road pieces (round HU2), the fill of its ground and cut strokes.
    surface: surface::PieceSurface,
    state: Option<HueState>,
    revision: u64,
}

impl FootprintHue {
    /// The state shown, `None` while hidden.
    pub(crate) fn state(&self) -> Option<HueState> {
        self.state
    }

    /// Changes whenever the mesh or the state does.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The fill's triangles without the padding.
    #[cfg(test)]
    pub(crate) fn filled(&self) -> &[Vector3] {
        &self.fill[..self.fill_used * 3]
    }

    /// Starts a new footprint: the strokes follow ([`Self::push_stroke`]), then
    /// [`Self::finish`].
    pub(crate) fn begin(&mut self) {
        self.path.clear();
        self.kinds.clear();
        self.surface.clear();
    }

    /// Adds the preview's road pieces the pushed path runs over (its spans, end caps, bends and
    /// junctions; not an existing road's span the edit only rebuilt): the fill of the path's
    /// ground and cut strokes becomes their drawn surface. Push the strokes first.
    pub(crate) fn push_pieces<'a>(&mut self, pieces: impl Iterator<Item = &'a PieceMesh>) {
        self.surface.push_pieces(&self.path, pieces);
    }

    /// The plan bounds `[min_x, min_z, max_x, max_z]` of the pushed pieces (what the drawn
    /// ground is read over), `None` without one.
    #[cfg(test)]
    pub(crate) fn surface_bounds(&self) -> Option<[f64; 4]> {
        self.surface.bounds()
    }

    /// Appends a stroke's points (in drawing order) of `kind`; a first point equal to the path's
    /// last in plan (a joint between strokes) is not repeated.
    pub(crate) fn push_stroke(&mut self, points: &[Vector3], kind: HueKind) {
        let skip = usize::from(
            self.path
                .last()
                .zip(points.first())
                .is_some_and(|(last, first)| last.x == first.x && last.z == first.z),
        );
        for &point in &points[skip.min(points.len())..] {
            if !self.path.is_empty() {
                self.kinds.push(kind);
            }
            self.path.push(point);
        }
    }

    /// Appends a stroke's points at the grade line from `heights[0]` to `heights[1]`
    /// ([`apply_grade_line`]) as a road: a refused graded draft that has no prepared stroke.
    pub(crate) fn push_graded(&mut self, points: &[Vector3], heights: [f32; 2]) {
        let start = self.path.len();
        self.push_stroke(points, HueKind::Road);
        // A skipped joint point stands for the stroke's first point: the grade line starts there.
        let joint = self.path.len() - start < points.len();
        apply_grade_line(&mut self.path[start - usize::from(joint)..], heights);
    }

    /// Builds the hue of the pushed path at `half_width_m` either side, in `state`, reading the
    /// ground of the path's strip from `ground` (world XZ to height) and the drawn ground under
    /// the pushed pieces from `drawn` (read at the graded surface's `GL0` lattice points). A path
    /// without horizontal length hides the hue. `O(path points + strip cells)` plus the pieces'
    /// ([`surface`]).
    pub(crate) fn finish(
        &mut self,
        half_width_m: f32,
        mut ground: impl FnMut(f32, f32) -> f32,
        drawn: impl FnMut(f64, f64) -> f64,
        state: HueState,
    ) {
        self.fill.clear();
        self.outline.clear();
        self.outline_starts.clear();
        let pieces = !self.surface.is_empty();
        if pieces {
            self.surface
                .build(f64::from(HUE_LIFT_M), drawn, &mut self.fill);
        }
        let path = &self.path;
        let mut tunnel_from: Option<usize> = None;
        for index in 0..path.len().saturating_sub(1) {
            let kind = self.kinds[index];
            if kind == HueKind::Tunnel {
                tunnel_from.get_or_insert(index);
            } else if let Some(from) = tunnel_from.take() {
                push_outline(
                    path,
                    from,
                    index,
                    half_width_m,
                    &mut ground,
                    &mut self.outline,
                    &mut self.outline_starts,
                );
            }
            let (a, b) = (path[index], path[index + 1]);
            // The drawn pieces replace a ground or cut stroke's strip.
            let drawn_road = pieces && kind == HueKind::Road;
            if kind == HueKind::Tunnel || drawn_road || plan_normal(a, b).is_none() {
                continue;
            }
            let (offset_a, offset_b) = (
                ribbon_offset(path, index, half_width_m),
                ribbon_offset(path, index + 1, half_width_m),
            );
            let corners = [a + offset_a, a - offset_a, b + offset_b, b - offset_b];
            // A bridge's hue reads only the ground under it, a road's the higher of both.
            let road = (kind == HueKind::Road).then_some([a.y, b.y]);
            push_cells(corners, road, half_width_m, &mut ground, &mut self.fill);
        }
        if let Some(from) = tunnel_from {
            let last = path.len() - 1;
            push_outline(
                path,
                from,
                last,
                half_width_m,
                &mut ground,
                &mut self.outline,
                &mut self.outline_starts,
            );
        }
        self.fill_used = self.fill.len() / 3;
        if self.fill_used == 0 && self.outline.is_empty() {
            self.clear();
            return;
        }
        if self.fill_used > 0 {
            let padded = self.fill_used.max(FILL_TRIANGLES_MIN).next_power_of_two();
            let collapsed = self.fill[0];
            self.fill.resize(padded * 3, collapsed);
        }
        self.fill_bytes.clear();
        for vertex in &self.fill {
            for coordinate in [vertex.x, vertex.y, vertex.z] {
                self.fill_bytes.extend_from_slice(&coordinate.to_le_bytes());
            }
        }
        self.state = Some(state);
        self.revision += 1;
    }

    /// Shows the same footprint in `state` (a batch's tier 2 settling, a commit refused). Nothing
    /// changes while hidden.
    pub(crate) fn set_state(&mut self, state: HueState) {
        if self.state.is_some_and(|shown| shown != state) {
            self.state = Some(state);
            self.revision += 1;
        }
    }

    /// Hides the hue.
    pub(crate) fn clear(&mut self) {
        self.fill.clear();
        self.fill_bytes.clear();
        self.fill_used = 0;
        self.outline.clear();
        self.outline_starts.clear();
        if self.state.take().is_some() {
            self.revision += 1;
        }
    }

    /// Hides the hue when `frame` has no draft and none waits for its acknowledgement (a
    /// cancelled draft, a committed one with no stroke after it). While a sent stroke waits,
    /// the hue stays with its preview; once the commit is acknowledged it goes with it, held
    /// on screen by the drawing side only until the commit's own chunks show.
    pub(crate) fn follow(&mut self, frame: &RoadToolFrame) {
        if frame.state.phase != Phase::AwaitingAck && frame.stroke.points.len() < 2 {
            self.clear();
        }
    }
}

// The distance between two points in plan.
fn plan_distance(a: Vector3, b: Vector3) -> f32 {
    (b.x - a.x).hypot(b.z - a.z)
}

// The hue's height over plan point `(x, z)`: the higher of the road there (`None` for a bridge's
// hue, which lies on the ground) and the ground, plus the lift.
fn hue_height(
    road_m: Option<f32>,
    ground: &mut impl FnMut(f32, f32) -> f32,
    x: f32,
    z: f32,
) -> f32 {
    let ground_m = ground(x, z);
    road_m.map_or(ground_m, |road_m| road_m.max(ground_m)) + HUE_LIFT_M
}

// Appends a segment's quad (`[left_a, right_a, left_b, right_b]`, the road at `road` heights at
// its two ends, linear between) as a grid of cells of at most `GROUND_STEP_M` along and
// `GROUND_STEP_ACROSS_M` across, each vertex at [`hue_height`], two triangles a cell. The grid
// interpolates the quad's corners bilinearly in plan, so the cells tile the quad exactly.
fn push_cells(
    [left_a, right_a, left_b, right_b]: [Vector3; 4],
    road: Option<[f32; 2]>,
    half_width_m: f32,
    ground: &mut impl FnMut(f32, f32) -> f32,
    out: &mut Vec<Vector3>,
) {
    let along_m = plan_distance(left_a, left_b).max(plan_distance(right_a, right_b));
    let along = (along_m / GROUND_STEP_M).ceil().max(1.0) as usize;
    let across = (2.0 * half_width_m / GROUND_STEP_ACROSS_M).ceil().max(1.0) as usize;
    let mut at = |u: usize, v: usize| {
        let (u, v) = (u as f32 / along as f32, v as f32 / across as f32);
        let a = left_a.lerp(right_a, v);
        let b = left_b.lerp(right_b, v);
        let (x, z) = (a.x + (b.x - a.x) * u, a.z + (b.z - a.z) * u);
        let road_m = road.map(|[from, to]| from + (to - from) * u);
        Vector3::new(x, hue_height(road_m, ground, x, z), z)
    };
    for u in 0..along {
        // One row of cells: the near and far edges' vertices, each read once.
        let mut near = at(u, 0);
        let mut far = at(u + 1, 0);
        for v in 0..across {
            let (near_next, far_next) = (at(u, v + 1), at(u + 1, v + 1));
            out.extend([near, near_next, far, far, near_next, far_next]);
            (near, far) = (near_next, far_next);
        }
    }
}

// Appends the outline of the tunnel run from path point `from` to `to` (its left edge forward,
// the right edge back, closed) as one strip, each edge cut every `GROUND_STEP_M` and lifted as
// the fill ([`hue_height`]: on the ground over the bore, on the road in its open approach).
#[allow(clippy::too_many_arguments)]
fn push_outline(
    path: &[Vector3],
    from: usize,
    to: usize,
    half_width_m: f32,
    ground: &mut impl FnMut(f32, f32) -> f32,
    outline: &mut Vec<Vector3>,
    starts: &mut Vec<u32>,
) {
    let first = outline.len();
    starts.push(first as u32);
    for index in from..to {
        let end = index + 1 == to;
        push_edge(path, (index, index + 1), half_width_m, end, ground, outline);
    }
    for index in (from..to).rev() {
        let end = index == from;
        push_edge(
            path,
            (index + 1, index),
            -half_width_m,
            end,
            ground,
            outline,
        );
    }
    outline.push(outline[first]);
}

// Appends one outline edge from path point `a`'s offset to `b`'s (left for a positive
// `half_width_m`, right for a negative one), points at most `GROUND_STEP_M` apart, without its
// last point (the next edge's first) unless `end` (the run's end, where the strip turns).
fn push_edge(
    path: &[Vector3],
    (a, b): (usize, usize),
    half_width_m: f32,
    end: bool,
    ground: &mut impl FnMut(f32, f32) -> f32,
    outline: &mut Vec<Vector3>,
) {
    let (from, to) = (
        path[a] + ribbon_offset(path, a, half_width_m),
        path[b] + ribbon_offset(path, b, half_width_m),
    );
    let steps = (plan_distance(from, to) / GROUND_STEP_M).ceil().max(1.0) as usize;
    for step in 0..steps + usize::from(end) {
        let t = step as f32 / steps as f32;
        let (x, z) = (from.x + (to.x - from.x) * t, from.z + (to.z - from.z) * t);
        let road_m = path[a].y + (path[b].y - path[a].y) * t;
        outline.push(Vector3::new(x, hue_height(Some(road_m), ground, x, z), z));
    }
}

mod surface;
pub(crate) use surface::graded_ground;
#[cfg(test)]
mod rows;
#[cfg(test)]
mod tests;
