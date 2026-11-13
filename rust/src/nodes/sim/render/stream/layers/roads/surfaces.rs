// SPDX-License-Identifier: GPL-2.0-only

//! A road chunk's surfaces in the `RenderingServer`'s own layout (`RENDER-52` part 2,
//! `docs/terrain.md` "Streaming core"): the bytes `ArrayMesh.add_surface_from_arrays` stores for
//! each of the chunk's layers, made in Rust off the main thread, so a frame only hands finished
//! buffers to `RenderingServer.mesh_add_surface`.
//!
//! The layout is Godot 4.7.2's uncompressed one, read back from `mesh_get_surface` and pinned
//! by the tests below with the engine's own bytes (`format_probe.gd`, `aabb_probe.gd`):
//! - `vertex_data`: the positions as f32 x 3 (stride 12), then at `12 n` a stride-8 stream of
//!   the normal, octahedral-encoded as two u16 truncated from `x * 65535`, and a tangent the
//!   engine generates from the normal (the format gains its tangent bit), encoded the same way
//!   with the sign folded into `y`. A vertex-only surface (the lining's shadow caster) has the
//!   positions alone.
//! - `attribute_data`: per vertex the colour as RGBA8 (truncated from `c as f64 * 255.0`), the
//!   UV as f32 x 2 and, on the road layer, `CUSTOM0` as f32 x 4 (stride 12 or 28).
//! - `aabb`: the positions' exact bounds, each extent raised to at least `1e-5`.
//!
//! Encoding is `O(vertices)` with one allocation a buffer, all on the caller's thread.

use crate::simulation::network::render::NetworkMeshData;
use godot::builtin::{Color, Vector2, Vector3, Vector4};

/// `RenderingServer.ARRAY_FORMAT_VERTEX`.
const FORMAT_VERTEX: u64 = 1;
/// `ARRAY_FORMAT_NORMAL`.
const FORMAT_NORMAL: u64 = 1 << 1;
/// `ARRAY_FORMAT_TANGENT`, which the engine adds whenever a surface has normals.
const FORMAT_TANGENT: u64 = 1 << 2;
/// `ARRAY_FORMAT_COLOR`.
const FORMAT_COLOR: u64 = 1 << 3;
/// `ARRAY_FORMAT_TEX_UV`.
const FORMAT_TEX_UV: u64 = 1 << 4;
/// `ARRAY_FORMAT_CUSTOM0`.
const FORMAT_CUSTOM0: u64 = 1 << 6;
/// `ARRAY_CUSTOM_RGBA_FLOAT << ARRAY_FORMAT_CUSTOM0_SHIFT`.
const FORMAT_CUSTOM0_RGBA_FLOAT: u64 = 7 << 13;
/// `ARRAY_FLAG_FORMAT_CURRENT_VERSION` (the engine stamps every new surface with it).
const FORMAT_VERSION: u64 = 1 << 35;
/// The smallest extent the engine gives a surface's AABB (`SMALL_VEC3`, `CMP_EPSILON`).
const SMALL_EXTENT: f32 = 0.00001;

/// A chunk's layers in payload order (`road_chunk_renderer.gd` `ROAD_CHUNK_LAYERS`).
pub(crate) const LAYER_COUNT: usize = 9;
/// The road layer, the one with `CUSTOM0` (crossing metres, paint age, lane offset).
const ROAD_LAYER: usize = 4;
/// The lining layer, which a shadow-only caster draws again (`ROAD-61`).
const LINING_LAYER: usize = 7;

/// One surface as `RenderingServer.mesh_add_surface` takes it (triangles, no index).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurface {
    /// The surface format bits.
    pub(crate) format: u64,
    /// Vertices.
    pub(crate) vertex_count: u32,
    /// Positions, then normals and tangents.
    pub(crate) vertex_data: Vec<u8>,
    /// Colours, UVs and `CUSTOM0`; empty for a vertex-only surface.
    pub(crate) attribute_data: Vec<u8>,
    /// The AABB: position then size.
    pub(crate) aabb: [f32; 6],
}

impl RoadSurface {
    /// Bytes handed to the server.
    pub(crate) fn bytes(&self) -> usize {
        self.vertex_data.len() + self.attribute_data.len()
    }
}

/// A chunk's encoded surfaces: each non-empty layer's, in layer order, and the lining's
/// vertex-only shadow caster when the chunk has a lining.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RoadChunkSurfaces {
    /// `(layer, surface)` for every non-empty layer, in layer order: the mesh's surface `i` is
    /// entry `i`.
    pub(crate) layers: Vec<(usize, RoadSurface)>,
    /// The lining's positions alone, for the shadow-only caster's own mesh.
    pub(crate) lining_shadow: Option<RoadSurface>,
}

impl RoadChunkSurfaces {
    /// Encodes every layer of `mesh`. `O(vertices)`.
    pub(crate) fn encode(mesh: &NetworkMeshData) -> Self {
        let mut layers = Vec::new();
        for layer in 0..LAYER_COUNT {
            let (vertices, normals, uvs, colors) = layer_arrays(mesh, layer);
            if vertices.is_empty() {
                continue;
            }
            let custom = (layer == ROAD_LAYER).then_some(mesh.road_crossings.as_slice());
            layers.push((layer, encode_layer(vertices, normals, uvs, colors, custom)));
        }
        let lining_shadow =
            (!mesh.lining_vertices.is_empty()).then(|| encode_positions(&mesh.lining_vertices));
        Self {
            layers,
            lining_shadow,
        }
    }
}

// A layer's arrays as the payload names them (`network_mesh_data_dict`).
fn layer_arrays(
    mesh: &NetworkMeshData,
    layer: usize,
) -> (&[Vector3], &[Vector3], &[Vector2], &[Color]) {
    match layer {
        0 => (&mesh.skirt_vertices, &mesh.skirt_normals, &mesh.skirt_uvs, &mesh.skirt_colors),
        1 => (&mesh.curb_vertices, &mesh.curb_normals, &mesh.curb_uvs, &mesh.curb_colors),
        2 => (
            &mesh.raised_step_vertices,
            &mesh.raised_step_normals,
            &mesh.raised_step_uvs,
            &mesh.raised_step_colors,
        ),
        3 => (
            &mesh.sidewalk_vertices,
            &mesh.sidewalk_normals,
            &mesh.sidewalk_uvs,
            &mesh.sidewalk_colors,
        ),
        ROAD_LAYER => (&mesh.road_vertices, &mesh.road_normals, &mesh.road_uvs, &mesh.road_colors),
        5 => (
            &mesh.marking_vertices,
            &mesh.marking_normals,
            &mesh.marking_uvs,
            &mesh.marking_colors,
        ),
        6 => (
            &mesh.concrete_vertices,
            &mesh.concrete_normals,
            &mesh.concrete_uvs,
            &mesh.concrete_colors,
        ),
        LINING_LAYER => (
            &mesh.lining_vertices,
            &mesh.lining_normals,
            &mesh.lining_uvs,
            &mesh.lining_colors,
        ),
        _ => (
            &mesh.median_grass_vertices,
            &mesh.median_grass_normals,
            &mesh.median_grass_uvs,
            &mesh.median_grass_colors,
        ),
    }
}

// A layer with normals, colours, UVs and, on the road layer, `CUSTOM0`. The payload is sealed
// (`NetworkMeshData::seal`), so its arrays have one length.
fn encode_layer(
    vertices: &[Vector3],
    normals: &[Vector3],
    uvs: &[Vector2],
    colors: &[Color],
    custom: Option<&[Vector4]>,
) -> RoadSurface {
    let count = vertices.len();
    let mut vertex_data = Vec::with_capacity(count * 20);
    write_positions(&mut vertex_data, vertices);
    for normal in normals {
        let [nx, ny] = octahedron_encode(*normal);
        let [tx, ty] = octahedron_tangent_encode(generated_tangent(*normal));
        let mut tangent = [unorm16(tx), unorm16(ty)];
        // (0, 1) and (1, 1) decode alike; the engine keeps (0, 1) out of its compression test.
        if tangent == [0, u16::MAX] {
            tangent[0] = u16::MAX;
        }
        for value in [unorm16(nx), unorm16(ny), tangent[0], tangent[1]] {
            vertex_data.extend_from_slice(&value.to_le_bytes());
        }
    }
    let stride = if custom.is_some() { 28 } else { 12 };
    let mut attribute_data = Vec::with_capacity(count * stride);
    for at in 0..count {
        let color = colors[at];
        attribute_data.extend([color.r, color.g, color.b, color.a].map(unorm8));
        let uv = uvs[at];
        for value in [uv.x, uv.y] {
            attribute_data.extend_from_slice(&value.to_le_bytes());
        }
        if let Some(custom) = custom {
            let value = custom[at];
            for lane in [value.x, value.y, value.z, value.w] {
                attribute_data.extend_from_slice(&lane.to_le_bytes());
            }
        }
    }
    let mut format =
        FORMAT_VERSION | FORMAT_VERTEX | FORMAT_NORMAL | FORMAT_TANGENT | FORMAT_COLOR | FORMAT_TEX_UV;
    if custom.is_some() {
        format |= FORMAT_CUSTOM0 | FORMAT_CUSTOM0_RGBA_FLOAT;
    }
    RoadSurface {
        format,
        vertex_count: count as u32,
        vertex_data,
        attribute_data,
        aabb: bounds(vertices),
    }
}

// A vertex-only surface (the lining's shadow caster).
fn encode_positions(vertices: &[Vector3]) -> RoadSurface {
    let mut vertex_data = Vec::with_capacity(vertices.len() * 12);
    write_positions(&mut vertex_data, vertices);
    RoadSurface {
        format: FORMAT_VERSION | FORMAT_VERTEX,
        vertex_count: vertices.len() as u32,
        vertex_data,
        attribute_data: Vec::new(),
        aabb: bounds(vertices),
    }
}

fn write_positions(out: &mut Vec<u8>, vertices: &[Vector3]) {
    for vertex in vertices {
        for value in [vertex.x, vertex.y, vertex.z] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

// The positions' bounds; an extent below the engine's smallest is raised to it.
fn bounds(vertices: &[Vector3]) -> [f32; 6] {
    let first = vertices[0];
    let (mut low, mut high) = ([first.x, first.y, first.z], [first.x, first.y, first.z]);
    for vertex in &vertices[1..] {
        for (axis, value) in [vertex.x, vertex.y, vertex.z].into_iter().enumerate() {
            if value < low[axis] {
                low[axis] = value;
            }
            if value > high[axis] {
                high[axis] = value;
            }
        }
    }
    let size = |axis: usize| {
        let extent = high[axis] - low[axis];
        if extent < SMALL_EXTENT { SMALL_EXTENT } else { extent }
    };
    [low[0], low[1], low[2], size(0), size(1), size(2)]
}

// `(uint16_t)CLAMP(x * 65535, 0, 65535)` in f32, truncated (a NaN stays NaN through the clamp and
// reads 0, as the engine's conversion does on x86-64 and as Rust's saturating cast does).
fn unorm16(value: f32) -> u16 {
    (value * 65535.0).clamp(0.0, 65535.0) as u16
}

// `uint8_t(CLAMP(c * 255.0, 0.0, 255.0))` with the product in double precision, truncated.
fn unorm8(value: f32) -> u8 {
    (f64::from(value) * 255.0).clamp(0.0, 255.0) as u8
}

// `Vector3::octahedron_encode`, in f32 as the engine computes it.
fn octahedron_encode(normal: Vector3) -> [f32; 2] {
    let sum = normal.x.abs() + normal.y.abs() + normal.z.abs();
    let (x, y, z) = (normal.x / sum, normal.y / sum, normal.z / sum);
    let sign = |value: f32| if value >= 0.0 { 1.0 } else { -1.0 };
    let (ox, oy) = if z >= 0.0 {
        (x, y)
    } else {
        ((1.0 - y.abs()) * sign(x), (1.0 - x.abs()) * sign(y))
    };
    [ox * 0.5 + 0.5, oy * 0.5 + 0.5]
}

// `Vector3::octahedron_tangent_encode(1.0)`: `y` biased off zero and folded into the upper half.
fn octahedron_tangent_encode(tangent: Vector3) -> [f32; 2] {
    let bias = 1.0 / 32767.0_f32;
    let [x, y] = octahedron_encode(tangent);
    // `MAX(y, bias)`: a NaN `y` (a zero tangent) gives the bias.
    let y = if y > bias { y } else { bias };
    [x, y * 0.5 + 0.5]
}

// The tangent the engine makes for a surface given normals but no tangents:
// `Vector3(n.z, -n.x, n.y).cross(n.normalized()).normalized()`.
fn generated_tangent(normal: Vector3) -> Vector3 {
    let a = [normal.z, -normal.x, normal.y];
    let b = normalized([normal.x, normal.y, normal.z]);
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let [x, y, z] = normalized(cross);
    Vector3::new(x, y, z)
}

// `Vector3::normalized`: zero stays zero.
fn normalized(v: [f32; 3]) -> [f32; 3] {
    let length_sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if length_sq == 0.0 {
        return [0.0; 3];
    }
    let length = length_sq.sqrt();
    [v[0] / length, v[1] / length, v[2] / length]
}

#[cfg(test)]
mod tests;
