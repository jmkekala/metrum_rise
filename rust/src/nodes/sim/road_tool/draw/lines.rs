// SPDX-License-Identifier: GPL-2.0-only

//! The tool's line quads and the upload byte layouts (`ROAD-112` rounds V, V2, R2-2, R2-3).
//!
//! Gizmo strips, the hue's tunnel outlines and the batch ghost's edges are drawn as quads that
//! `road_tool_line.gdshader` widens to a constant on-screen width (as `route_ribbon.gdshader`
//! does): four vertices a segment, the vertex buffer's positions and the attribute buffer's
//! interleaved `[other end x, y, z, side * (dash metres + 1), style]` (UV, UV2 and CUSTOM0's red,
//! 20 bytes a vertex; the side is the sign, the dash station the magnitude less one; the style
//! picks the shader's colour, width and dash: [`LineStyle`]), padded with collapsed quads to a
//! power-of-two segment count (at least 16; none without strips) so a drag keeps the surface's
//! layout and uploads in place. Every style in one surface (round V2: one upload a frame instead
//! of one a style).

use crate::nodes::sim::road_tool::frame::{GizmoHandleKind, GizmoLineClass};
use godot::builtin::Vector3;

// Minimum quads a line surface holds.
const LINE_QUADS_MIN: usize = 16;
/// Floats a line vertex's attributes take.
pub(crate) const LINE_ATTRIBUTES: usize = 5;
/// The bytes of a position in a vertex buffer: three little-endian `f32`.
#[cfg(test)]
pub(crate) const VERTEX_BYTES: usize = 12;

/// The line looks `road_tool_line.gdshader` draws, as the attributes number them (its palette
/// arrays in `road_tool_gizmos.gd`'s `PALETTE` follow this order).
#[derive(Clone, Copy)]
pub(crate) enum LineStyle {
    /// The draft's centreline.
    Stroke = 0,
    /// Every guide line in one look (round V item 6; a height handle's stem too, round V2); since
    /// round UI1 the tangent legs, control polygons and stems, the alignment guides having
    /// their own.
    Guide = 1,
    /// The zoning guide's outside (item 9).
    ZoningOutside = 2,
    /// Its inside.
    ZoningInside = 3,
    /// The grid curbs (item 8, drawn to stand out on the preview road's own edges).
    GridCurb = 4,
    /// A batch ghost's edges, tier 1 accepting the stroke (round V2).
    GhostEdge = 5,
    /// A batch ghost's edges, tier 1 refusing the stroke.
    GhostRefusedEdge = 6,
    /// An end's ground marker: its vertical line and its ring on the ground (round H2).
    GroundMarker = 7,
    /// A tunnel run's outline in the footprint hue (round HU), the draft accepted.
    HueOutlineValid = 8,
    /// The same, the draft still checked.
    HueOutlineChecking = 9,
    /// The same, the draft refused.
    HueOutlineRefused = 10,
    /// The alignment guide that took the cursor and the angle lock's ray (round UI1: thicker and
    /// more opaque than the shared guide look).
    Alignment = 11,
    /// A guide line near the cursor that has not taken it (round UI1), faint.
    GuideNear = 12,
    /// An end's corner arc (round UI1), or its square at a right angle (round UI2).
    CornerArc = 13,
    /// The preview's junction outline (`ROAD-118` rounds JS5b, JS5e): its own look, thicker and
    /// lighter than the hue outline so it reads on the hue's fill and on bare ground.
    JoinOutline = 14,
    /// A `roads_overlap` refusal's overlapped stretch (rounds JS5b, JS5e): the outline's look in
    /// a matching red.
    JoinOverlap = 15,
}

/// A gizmo strip class's look.
pub(crate) fn line_style(class: GizmoLineClass) -> LineStyle {
    match class {
        GizmoLineClass::Draft => LineStyle::Stroke,
        GizmoLineClass::Guide | GizmoLineClass::Angle => LineStyle::Alignment,
        GizmoLineClass::GuideNear => LineStyle::GuideNear,
        GizmoLineClass::CornerArc => LineStyle::CornerArc,
        GizmoLineClass::Tangent | GizmoLineClass::HeightStem => LineStyle::Guide,
        GizmoLineClass::ZoningOutside => LineStyle::ZoningOutside,
        GizmoLineClass::ZoningInside => LineStyle::ZoningInside,
        GizmoLineClass::GridCurb => LineStyle::GridCurb,
        GizmoLineClass::GroundMarker => LineStyle::GroundMarker,
    }
}

/// A handle kind's code: the handle surface's UV (`road_tool_handle.gdshader`'s palette index).
pub(crate) fn handle_kind_code(kind: GizmoHandleKind) -> i32 {
    match kind {
        GizmoHandleKind::Endpoint => 0,
        GizmoHandleKind::Control => 1,
        GizmoHandleKind::ChainStart => 2,
        GizmoHandleKind::Cursor => 3,
        GizmoHandleKind::Joint => 4,
        // A height grip draws as a handle, in round V's look (rounds I, V2).
        GizmoHandleKind::Handle | GizmoHandleKind::HeightGrip => 5,
    }
}

// The quads' segment count before padding.
fn strip_segments(points: &[Vector3], starts: &[u32]) -> usize {
    (0..starts.len())
        .map(|index| strip(points, starts, index).len().saturating_sub(1))
        .sum()
}

/// The padded quad count of strips (`points` back to back, `starts` each strip's first point):
/// none without segments. `O(strips)`.
pub(crate) fn strip_quad_count(points: &[Vector3], starts: &[u32]) -> usize {
    let segments = strip_segments(points, starts);
    if segments == 0 {
        0
    } else {
        segments.max(LINE_QUADS_MIN).next_power_of_two()
    }
}

// Strip `index`'s points.
fn strip<'a>(points: &'a [Vector3], starts: &[u32], index: usize) -> &'a [Vector3] {
    let start = starts[index] as usize;
    let end = starts
        .get(index + 1)
        .map_or(points.len(), |&end| end as usize);
    &points[start..end]
}

/// Strips as line quads in reused buffers (see the module header).
#[derive(Default)]
pub(crate) struct LineQuadBuffers {
    /// The quads' positions, four a segment, padded.
    pub(crate) vertices: Vec<Vector3>,
    /// Their interleaved attributes, [`LINE_ATTRIBUTES`] floats a vertex.
    pub(crate) attributes: Vec<f32>,
}

impl LineQuadBuffers {
    /// The strips' quads into the buffers, each strip in the look `style_of` its index, the
    /// padding collapsed at the origin. `O(points)`; allocates nothing once the buffers have
    /// grown. Only the padding is zeroed (round R2-9): the quads overwrite the rest.
    pub(crate) fn fill(
        &mut self,
        points: &[Vector3],
        starts: &[u32],
        style_of: impl Fn(usize) -> LineStyle,
    ) {
        let capacity = strip_quad_count(points, starts);
        self.vertices.resize(capacity * 4, Vector3::ZERO);
        self.attributes.resize(capacity * 4 * LINE_ATTRIBUTES, 0.0);
        let mut quad = 0;
        for index in 0..starts.len() {
            let style = style_of(index) as u8 as f32;
            let mut dash = 0.0;
            for pair in strip(points, starts, index).windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let length = a.distance_to(b);
                // Each end twice, one vertex per side; the side's sign is taken from the start's
                // view, so it flips at the far end (whose other end points back).
                for (corner, (point, other, side, at)) in [
                    (a, b, 1.0, dash),
                    (a, b, -1.0, dash),
                    (b, a, -1.0, dash + length),
                    (b, a, 1.0, dash + length),
                ]
                .into_iter()
                .enumerate()
                {
                    let vertex = quad * 4 + corner;
                    self.vertices[vertex] = point;
                    self.attributes[vertex * LINE_ATTRIBUTES..(vertex + 1) * LINE_ATTRIBUTES]
                        .copy_from_slice(&[other.x, other.y, other.z, side * (at + 1.0), style]);
                }
                dash += length;
                quad += 1;
            }
        }
        self.vertices[quad * 4..].fill(Vector3::ZERO);
        self.attributes[quad * 4 * LINE_ATTRIBUTES..].fill(0.0);
    }
}

/// Positions as a vertex buffer holds them: three little-endian `f32` each (the tests' reference
/// for the backends' own encoding, `backend::Bytes`, round R2-9). `O(vectors)`.
#[cfg(test)]
pub(crate) fn write_vectors(bytes: &mut Vec<u8>, vectors: &[Vector3]) {
    bytes.resize(vectors.len() * VERTEX_BYTES, 0);
    for (chunk, vector) in bytes.as_chunks_mut::<VERTEX_BYTES>().0.iter_mut().zip(vectors) {
        chunk[..4].copy_from_slice(&vector.x.to_le_bytes());
        chunk[4..8].copy_from_slice(&vector.y.to_le_bytes());
        chunk[8..].copy_from_slice(&vector.z.to_le_bytes());
    }
}

/// Floats as little-endian bytes (the tests' reference, as [`write_vectors`]). `O(floats)`.
#[cfg(test)]
pub(crate) fn write_floats(bytes: &mut Vec<u8>, floats: &[f32]) {
    bytes.resize(floats.len() * 4, 0);
    for (chunk, float) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(floats) {
        *chunk = float.to_le_bytes();
    }
}
