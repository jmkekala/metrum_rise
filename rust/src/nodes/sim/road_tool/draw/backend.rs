// SPDX-License-Identifier: GPL-2.0-only

//! The tool's drawing calls behind [`DrawBackend`] (`ROAD-112` round R2-3), the pattern of the
//! streaming core's `sim/render/stream/backend.rs`, whose trait is `MultiMesh`-only: the tool
//! draws mesh surfaces. [`super::engine::GodotBackend`] calls the `RenderingServer`; the
//! recorder (tests) keeps the commands and the state they leave. Main thread only.

use crate::nodes::sim::render::stream::layers::roads::surfaces::RoadSurface;

/// A backend object: a `RenderingServer` RID's id, or a recorder's number.
pub(crate) type Handle = u64;

/// `RenderingServer.ARRAY_FORMAT_VERTEX`.
const FORMAT_VERTEX: u64 = 1;
/// `ARRAY_FORMAT_TEX_UV`.
const FORMAT_TEX_UV: u64 = 1 << 4;
/// `ARRAY_FORMAT_TEX_UV2`.
const FORMAT_TEX_UV2: u64 = 1 << 5;
/// `ARRAY_FORMAT_CUSTOM0`.
const FORMAT_CUSTOM0: u64 = 1 << 6;
/// `ARRAY_FORMAT_INDEX`.
const FORMAT_INDEX: u64 = 1 << 12;
/// `ARRAY_CUSTOM_R_FLOAT << ARRAY_FORMAT_CUSTOM0_SHIFT`.
const FORMAT_CUSTOM0_R_FLOAT: u64 = 4 << 13;
/// `ARRAY_FLAG_FORMAT_CURRENT_VERSION` (the engine stamps every new surface with it).
const FORMAT_VERSION: u64 = 1 << 35;

/// The surfaces the tool draws, each the layout `ArrayMesh.add_surface_from_arrays` stored for
/// the arrays `road_tool_gizmos.gd` and `road_tool_hue.gd` built before round R2-3 (Godot
/// 4.7.2's uncompressed layout; a Godot test reads the server's copy back against it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceKind {
    /// Line quads (`road_tool_line.gdshader`): indexed triangles, positions (12 bytes a vertex),
    /// then UV, UV2 and `CUSTOM0` as one red float (20 bytes a vertex).
    Lines,
    /// Handles (`road_tool_handle.gdshader`): points, positions and the kind as UV (8 bytes).
    Points,
    /// A fill (the hue's, the batch ghost's): a triangle list of positions alone.
    Fill,
}

impl SurfaceKind {
    /// The surface's format bits.
    pub(crate) fn format(self) -> u64 {
        FORMAT_VERSION
            | FORMAT_VERTEX
            | match self {
                Self::Lines => {
                    FORMAT_TEX_UV
                        | FORMAT_TEX_UV2
                        | FORMAT_CUSTOM0
                        | FORMAT_CUSTOM0_R_FLOAT
                        | FORMAT_INDEX
                }
                Self::Points => FORMAT_TEX_UV,
                Self::Fill => 0,
            }
    }

    /// Whether the surface draws points (else triangles).
    pub(crate) fn points(self) -> bool {
        self == Self::Points
    }
}

/// One surface as the backend adds it: finished buffers in the server's layout.
pub(crate) struct Surface<'a> {
    /// Its layout.
    pub(crate) kind: SurfaceKind,
    /// Vertices.
    pub(crate) vertex_count: u32,
    /// The positions.
    pub(crate) vertices: &'a [u8],
    /// The attributes (empty for a fill).
    pub(crate) attributes: &'a [u8],
    /// The index buffer (two bytes an index up to 65,536 vertices, else four; empty unless
    /// [`SurfaceKind::Lines`]) and its index count.
    pub(crate) indices: &'a [u8],
    /// Indices.
    pub(crate) index_count: u32,
    /// The surface's material.
    pub(crate) material: Handle,
}

/// A buffer of a surface written in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Region {
    /// The vertex buffer (positions).
    Vertices,
    /// The attribute buffer.
    Attributes,
}

/// The tool's drawing calls. Every call is `O(bytes passed)` or `O(1)`.
pub(crate) trait DrawBackend {
    /// A new empty mesh.
    fn mesh_create(&mut self) -> Handle;
    /// An instance of `mesh` in the tool's scenario: hidden, casting no shadow, never culled.
    fn instance_create(&mut self, mesh: Handle) -> Handle;
    /// Appends `surface` to `mesh`.
    fn mesh_add_surface(&mut self, mesh: Handle, surface: &Surface);
    /// Removes every surface of `mesh`.
    fn mesh_clear(&mut self, mesh: Handle);
    /// Rewrites `region` of `mesh`'s surface `surface` from its start with `bytes`.
    fn mesh_update_region(&mut self, mesh: Handle, surface: u32, region: Region, bytes: &[u8]);
    /// Sets the material of `mesh`'s surface `surface`.
    fn mesh_set_material(&mut self, mesh: Handle, surface: u32, material: Handle);
    /// An instance of `mesh` as a road chunk's node draws it (round R2-4a, the preview's
    /// overlay chunks): hidden, casting no shadow (the roads' receiver-only policy), culled by
    /// its mesh's bounds.
    fn instance_create_plain(&mut self, mesh: Handle) -> Handle;
    /// Appends a road chunk layer's encoded surface (`surfaces.rs`) to `mesh`, drawn with
    /// `material`.
    fn mesh_add_road_surface(&mut self, mesh: Handle, surface: &RoadSurface, material: Handle);
    /// Places `instance` at `origin` (identity basis).
    fn instance_set_origin(&mut self, instance: Handle, origin: [f32; 3]);
    /// Shows or hides `instance`.
    fn instance_set_visible(&mut self, instance: Handle, visible: bool);
    /// Where the line material's dashes keep their on-screen length (`dash_focus`).
    fn set_dash_focus(&mut self, focus: [f32; 3]);
    /// Frees a mesh or an instance.
    fn free(&mut self, handle: Handle);
}
