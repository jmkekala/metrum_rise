// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's drawing (`ROAD-112` round R2-3): the gizmos, the footprint hue (and a
//! commit's hue held with its preview) and the batch ghost, drawn through the `RenderingServer`
//! by Rust instead of `road_tool_gizmos.gd` and `road_tool_hue.gd`.
//!
//! [`ToolDraw`] owns one mesh and one instance each for the gizmos, the ghost and two hues (the
//! current one and a commit's, held until the commit's chunks install), made once when the
//! tool's drawing setup attaches (`road_tool_attach_draw`) and freed when it detaches or drops.
//! Each overlay keeps its surfaces' layout (their vertex counts): while a new draw has the same
//! layout its buffers are rewritten in place (`mesh_surface_update_*_region`); a new layout
//! rebuilds the surfaces from the finished bytes (`mesh_add_surface`, no array conversion). The
//! gizmos' and the hue's buffers are padded to power-of-two counts, so a drag mostly writes in
//! place. The bytes are built here in kept buffers: a draw allocates nothing once they have grown
//! (a rebuild hands the engine fresh arrays).
//!
//! Behind [`backend::DrawBackend`]: [`engine::GodotBackend`] for the game, a recorder in tests.
//! The palette and the materials stay `road_tool_gizmos.gd`'s (looks belong to the theme); Rust
//! gets them at attach. Main thread only.

pub(crate) mod backend;
pub(crate) mod engine;
pub(crate) mod lines;
#[cfg(test)]
pub(crate) mod recorder;
#[cfg(test)]
mod tests;

use crate::nodes::sim::road_tool::frame::Gizmos;
use crate::nodes::sim::road_tool::hue::{FootprintHue, HueState};
use backend::{DrawBackend, Handle, Region, Surface, SurfaceKind};
use godot::builtin::Vector3;
use lines::{
    LineQuadBuffers, LineStyle, VERTEX_BYTES, handle_kind_code, line_style, write_floats,
    write_vectors,
};

/// The materials the tool draws with, as backend handles.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DrawMaterials {
    /// Every line look (`road_tool_line.gdshader`, no depth test): the gizmos' lines and the
    /// batch ghost's edges.
    pub(crate) line: Handle,
    /// The same looks with the depth test (`road_tool_line_depth.gdshader`): the hue's tunnel
    /// outlines.
    pub(crate) depth_line: Handle,
    /// The handles (`road_tool_handle.gdshader`).
    pub(crate) handle: Handle,
    /// The hue's fill by state: valid, checking, refused (hatched).
    pub(crate) hue: [Handle; 3],
    /// The batch ghost's fill by tier 1's verdict: accepted, refused.
    pub(crate) ghost_fill: [Handle; 2],
}

/// Tier 1's ghost of a batch pose (rounds G1, V2), what [`ToolDraw::draw_ghost`] draws: every
/// stroke's ribbon at its road's width, its verdict, and the ribbons' edges. Reused pose to pose.
#[derive(Default)]
pub(crate) struct Ghost {
    /// The ribbons, a triangle list, stroke after stroke.
    pub(crate) ribbon: Vec<Vector3>,
    /// Each stroke's first ribbon vertex.
    pub(crate) starts: Vec<u32>,
    /// Tier 1's verdict each stroke.
    pub(crate) valid: Vec<bool>,
    /// The ribbons' edges, two strips a stroke, back to back.
    pub(crate) edges: Vec<Vector3>,
    /// Each edge strip's first point.
    pub(crate) edge_starts: Vec<u32>,
}

impl Ghost {
    /// Empties the ghost, keeping its buffers.
    pub(crate) fn clear(&mut self) {
        self.ribbon.clear();
        self.starts.clear();
        self.valid.clear();
        self.edges.clear();
        self.edge_starts.clear();
    }
}

/// One overlay's drawn state, for tests and probes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OverlayState {
    /// Its mesh.
    pub(crate) mesh: Handle,
    /// Whether its instance is shown.
    pub(crate) visible: bool,
    /// Its surfaces.
    pub(crate) surfaces: u32,
}

// The surfaces an overlay holds at most.
const PARTS: usize = 3;

// One mesh with its instance, and the layout and materials of its surfaces.
#[derive(Default)]
struct Overlay {
    mesh: Handle,
    instance: Handle,
    // Each part's vertex count (0: no surface) as drawn, `None` while the mesh is empty.
    layout: Option<[usize; PARTS]>,
    materials: [Handle; PARTS],
    visible: bool,
}

// A part of an overlay's draw: one surface's buffers, absent while it has no vertices.
struct Part<'a> {
    kind: SurfaceKind,
    vertex_count: usize,
    vertices: &'a [u8],
    attributes: &'a [u8],
    indices: &'a [u8],
    material: Handle,
}

impl Overlay {
    fn create(backend: &mut impl DrawBackend) -> Self {
        let mesh = backend.mesh_create();
        let instance = backend.instance_create(mesh);
        Self {
            mesh,
            instance,
            ..Self::default()
        }
    }

    // Draws `parts`: in place while the layout holds, else the surfaces rebuilt; shown when any
    // part has vertices. `O(bytes)`.
    fn show(&mut self, backend: &mut impl DrawBackend, parts: &[Part]) {
        let mut layout = [0; PARTS];
        for (count, part) in layout.iter_mut().zip(parts) {
            *count = part.vertex_count;
        }
        let present = parts.iter().enumerate().filter(|(_, part)| part.vertex_count > 0);
        if self.layout == Some(layout) {
            for (surface, (index, part)) in present.enumerate() {
                let surface = surface as u32;
                backend.mesh_update_region(self.mesh, surface, Region::Vertices, part.vertices);
                if !part.attributes.is_empty() {
                    backend.mesh_update_region(
                        self.mesh,
                        surface,
                        Region::Attributes,
                        part.attributes,
                    );
                }
                if self.materials[index] != part.material {
                    backend.mesh_set_material(self.mesh, surface, part.material);
                    self.materials[index] = part.material;
                }
            }
        } else {
            self.empty(backend);
            for (index, part) in present {
                backend.mesh_add_surface(
                    self.mesh,
                    &Surface {
                        kind: part.kind,
                        vertex_count: part.vertex_count as u32,
                        vertices: part.vertices,
                        attributes: part.attributes,
                        indices: part.indices,
                        index_count: (part.indices.len() / index_size(part.vertex_count)) as u32,
                        material: part.material,
                    },
                );
                self.materials[index] = part.material;
            }
            self.layout = Some(layout);
        }
        self.set_visible(backend, layout.iter().any(|&count| count > 0));
    }

    // Drops the surfaces and hides the instance.
    fn clear(&mut self, backend: &mut impl DrawBackend) {
        self.empty(backend);
        self.set_visible(backend, false);
    }

    fn empty(&mut self, backend: &mut impl DrawBackend) {
        if self.layout.take().is_some_and(|layout| layout.iter().any(|&count| count > 0)) {
            backend.mesh_clear(self.mesh);
        }
    }

    fn set_visible(&mut self, backend: &mut impl DrawBackend, visible: bool) {
        if self.visible != visible {
            self.visible = visible;
            backend.instance_set_visible(self.instance, visible);
        }
    }

    fn free(&self, backend: &mut impl DrawBackend) {
        backend.free(self.instance);
        backend.free(self.mesh);
    }

    fn state(&self) -> OverlayState {
        OverlayState {
            mesh: self.mesh,
            visible: self.visible,
            surfaces: self
                .layout
                .map_or(0, |layout| layout.iter().filter(|&&count| count > 0).count() as u32),
        }
    }
}

// The bytes an index takes for a surface of `vertex_count` vertices (the engine's rule: 16-bit
// up to 65,536 vertices).
fn index_size(vertex_count: usize) -> usize {
    if vertex_count <= 1 << 16 { 2 } else { 4 }
}

// Two triangles a quad of four vertices (`a+, a-, b+, b-`), as index bytes: one buffer per index
// size, grown to the largest count met (a smaller count's indices are its prefix).
#[derive(Default)]
struct QuadIndices {
    short: Vec<u8>,
    long: Vec<u8>,
}

impl QuadIndices {
    // The index bytes of `vertex_count` quad vertices. `O(new quads)`.
    fn of(&mut self, vertex_count: usize) -> &[u8] {
        let size = index_size(vertex_count);
        let buffer = if size == 2 {
            &mut self.short
        } else {
            &mut self.long
        };
        let quads = vertex_count / 4;
        for quad in buffer.len() / (6 * size)..quads {
            let base = quad * 4;
            for index in [base, base + 1, base + 2, base + 2, base + 1, base + 3] {
                if size == 2 {
                    buffer.extend_from_slice(&(index as u16).to_le_bytes());
                } else {
                    buffer.extend_from_slice(&(index as u32).to_le_bytes());
                }
            }
        }
        &buffer[..quads * 6 * size]
    }
}

// Line quads and their bytes, kept.
#[derive(Default)]
struct LineBytes {
    quads: LineQuadBuffers,
    vertices: Vec<u8>,
    attributes: Vec<u8>,
}

impl LineBytes {
    // Strips as quads, each strip in the look `style_of` its index. `O(points)`.
    fn fill(&mut self, points: &[Vector3], starts: &[u32], style_of: impl Fn(usize) -> LineStyle) {
        self.quads.fill(points, starts, style_of);
        write_vectors(&mut self.vertices, &self.quads.vertices);
        write_floats(&mut self.attributes, &self.quads.attributes);
    }

    fn vertex_count(&self) -> usize {
        self.quads.vertices.len()
    }
}

/// The tool's drawing: the gizmos, the hue (and a held commit's), the batch ghost, over a
/// backend. See the module header.
pub(crate) struct ToolDraw<B: DrawBackend> {
    backend: B,
    materials: DrawMaterials,
    gizmos: Overlay,
    hues: [Overlay; 2],
    // The current hue's overlay; the other holds a commit's hue until its chunks install.
    hue: usize,
    ghost: Overlay,
    indices: QuadIndices,
    gizmo_lines: LineBytes,
    handles: Vec<u8>,
    handle_kinds: Vec<u8>,
    hue_outline: LineBytes,
    ghost_fills: [Vec<Vector3>; 2],
    ghost_fill_bytes: [Vec<u8>; 2],
    ghost_edges: LineBytes,
}

impl<B: DrawBackend> ToolDraw<B> {
    /// Makes the meshes and instances (hidden) on `backend`, drawing with `materials`.
    pub(crate) fn new(mut backend: B, materials: DrawMaterials) -> Self {
        let gizmos = Overlay::create(&mut backend);
        let hues = [Overlay::create(&mut backend), Overlay::create(&mut backend)];
        let ghost = Overlay::create(&mut backend);
        Self {
            backend,
            materials,
            gizmos,
            hues,
            hue: 0,
            ghost,
            indices: QuadIndices::default(),
            gizmo_lines: LineBytes::default(),
            handles: Vec::new(),
            handle_kinds: Vec::new(),
            hue_outline: LineBytes::default(),
            ghost_fills: Default::default(),
            ghost_fill_bytes: Default::default(),
            ghost_edges: LineBytes::default(),
        }
    }

    /// Draws `gizmos`: one line surface of quads in every look and one point surface of the
    /// handles (round V2), the dashes kept at `focus`'s depth. `O(gizmo points)`; allocates
    /// nothing once the buffers have grown, unless the layout changed.
    pub(crate) fn draw_gizmos(&mut self, gizmos: &Gizmos, focus: Vector3) {
        self.gizmo_lines
            .fill(&gizmos.line_points, &gizmos.line_starts, |strip| {
                line_style(gizmos.line_classes[strip])
            });
        write_vectors(&mut self.handles, &gizmos.handles);
        // Each point's kind as its UV.
        let kinds = &mut self.handle_kinds;
        kinds.clear();
        kinds.resize(gizmos.handle_kinds.len() * 8, 0);
        for (chunk, kind) in kinds.as_chunks_mut::<8>().0.iter_mut().zip(&gizmos.handle_kinds) {
            chunk[..4].copy_from_slice(&(handle_kind_code(*kind) as f32).to_le_bytes());
            chunk[4..].copy_from_slice(&0.0f32.to_le_bytes());
        }
        self.backend.set_dash_focus(focus.to_array());
        let lines = self.gizmo_lines.vertex_count();
        let parts = [
            Part {
                kind: SurfaceKind::Lines,
                vertex_count: lines,
                vertices: &self.gizmo_lines.vertices,
                attributes: &self.gizmo_lines.attributes,
                indices: self.indices.of(lines),
                material: self.materials.line,
            },
            Part {
                kind: SurfaceKind::Points,
                vertex_count: gizmos.handles.len(),
                vertices: &self.handles,
                attributes: &self.handle_kinds,
                indices: &[],
                material: self.materials.handle,
            },
        ];
        self.gizmos.show(&mut self.backend, &parts);
    }

    /// Hides the gizmos.
    pub(crate) fn clear_gizmos(&mut self) {
        self.gizmos.clear(&mut self.backend);
    }

    /// Draws `hue` as the current hue: its fill in its state's material (with the depth test,
    /// round HU) and its tunnel runs' outlines as depth-tested line quads in the state's look;
    /// hidden without a state. `O(fill + outline points)`.
    pub(crate) fn draw_hue(&mut self, hue: &FootprintHue) {
        let overlay = &mut self.hues[self.hue];
        let Some(state) = hue.state() else {
            overlay.clear(&mut self.backend);
            return;
        };
        let style = match state {
            HueState::Valid => LineStyle::HueOutlineValid,
            HueState::Checking => LineStyle::HueOutlineChecking,
            HueState::Refused => LineStyle::HueOutlineRefused,
        };
        self.hue_outline
            .fill(&hue.outline, &hue.outline_starts, |_| style);
        let outline = self.hue_outline.vertex_count();
        let parts = [
            Part {
                kind: SurfaceKind::Fill,
                vertex_count: hue.fill.len(),
                vertices: &hue.fill_bytes,
                attributes: &[],
                indices: &[],
                material: self.materials.hue[state as usize],
            },
            Part {
                kind: SurfaceKind::Lines,
                vertex_count: outline,
                vertices: &self.hue_outline.vertices,
                attributes: &self.hue_outline.attributes,
                indices: self.indices.of(outline),
                material: self.materials.depth_line,
            },
        ];
        overlay.show(&mut self.backend, &parts);
    }

    /// Hides the current hue.
    pub(crate) fn clear_hue(&mut self) {
        self.hues[self.hue].clear(&mut self.backend);
    }

    /// An accepted commit's hue stays drawn with its preview until the commit's chunks install
    /// ([`Self::release_held_hue`]): the current hue becomes the held one (a hue held before goes)
    /// and the next draw starts an empty one.
    pub(crate) fn hold_hue(&mut self) {
        self.release_held_hue();
        self.hue = 1 - self.hue;
    }

    /// The held commit's hue goes (its chunks installed, or the preview was reset).
    pub(crate) fn release_held_hue(&mut self) {
        self.hues[1 - self.hue].clear(&mut self.backend);
    }

    /// Draws tier 1's `ghost` (F10: split by verdict here): one fill surface per verdict that has
    /// strokes (accepted, refused), each in its fill material, and one surface of the ribbons'
    /// edges in the ghost's line looks. `O(ribbon + edge points)`.
    pub(crate) fn draw_ghost(&mut self, ghost: &Ghost) {
        for fill in &mut self.ghost_fills {
            fill.clear();
        }
        for (stroke, &start) in ghost.starts.iter().enumerate() {
            let end = ghost
                .starts
                .get(stroke + 1)
                .map_or(ghost.ribbon.len(), |&end| end as usize);
            let refused = !ghost.valid.get(stroke).copied().unwrap_or(false);
            self.ghost_fills[usize::from(refused)].extend_from_slice(&ghost.ribbon[start as usize..end]);
        }
        for (bytes, fill) in self.ghost_fill_bytes.iter_mut().zip(&self.ghost_fills) {
            write_vectors(bytes, fill);
        }
        // Two edge strips a stroke, in the ghost's look by tier 1's verdict.
        self.ghost_edges
            .fill(&ghost.edges, &ghost.edge_starts, |strip| {
                if ghost.valid.get(strip / 2).copied().unwrap_or(false) {
                    LineStyle::GhostEdge
                } else {
                    LineStyle::GhostRefusedEdge
                }
            });
        let edges = self.ghost_edges.vertex_count();
        let fill = |verdict: usize| Part {
            kind: SurfaceKind::Fill,
            vertex_count: self.ghost_fill_bytes[verdict].len() / VERTEX_BYTES,
            vertices: &self.ghost_fill_bytes[verdict],
            attributes: &[],
            indices: &[],
            material: self.materials.ghost_fill[verdict],
        };
        let parts = [
            fill(0),
            fill(1),
            Part {
                kind: SurfaceKind::Lines,
                vertex_count: edges,
                vertices: &self.ghost_edges.vertices,
                attributes: &self.ghost_edges.attributes,
                indices: self.indices.of(edges),
                material: self.materials.line,
            },
        ];
        self.ghost.show(&mut self.backend, &parts);
    }

    /// Hides the batch ghost.
    pub(crate) fn clear_ghost(&mut self) {
        self.ghost.clear(&mut self.backend);
    }

    /// The overlays' drawn state: the gizmos, the current hue, the held hue, the ghost.
    pub(crate) fn state(&self) -> [OverlayState; 4] {
        [
            self.gizmos.state(),
            self.hues[self.hue].state(),
            self.hues[1 - self.hue].state(),
            self.ghost.state(),
        ]
    }

    #[cfg(test)]
    pub(crate) fn backend(&self) -> &B {
        &self.backend
    }

    #[cfg(test)]
    pub(crate) fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}

impl<B: DrawBackend> Drop for ToolDraw<B> {
    // Frees every instance and mesh the drawing made (the materials are the setup's).
    fn drop(&mut self) {
        for overlay in [&self.gizmos, &self.hues[0], &self.hues[1], &self.ghost] {
            overlay.free(&mut self.backend);
        }
    }
}
