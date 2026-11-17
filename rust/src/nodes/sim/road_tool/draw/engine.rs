// SPDX-License-Identifier: GPL-2.0-only

//! The `RenderingServer` backend of the tool's drawing (`ROAD-112` round R2-3). It keeps the
//! materials `road_tool_gizmos.gd` made (alive while the drawing is attached), the scenario the
//! instances draw in, upload arrays by length (the layouts are padded, so few lengths recur) and
//! one surface dictionary per surface kind with its keys made once, so neither an in-place write
//! nor a rebuild allocates on this side once a length has been met. Main thread only.

use super::backend::{DrawBackend, Handle, Region, Surface, SurfaceKind};
use crate::nodes::sim::render::stream::layers::roads::surfaces::RoadSurface;
use godot::classes::rendering_server::ShadowCastingSetting;
use godot::classes::{Material, RenderingServer, ShaderMaterial};
use godot::obj::Singleton;
use godot::prelude::{
    Aabb, Basis, Gd, PackedByteArray, Rid, StringName, ToGodot, Transform3D, VarDictionary,
    Variant, Vector3, Vector4,
};

/// `RenderingServer.PRIMITIVE_POINTS`.
const PRIMITIVE_POINTS: i64 = 0;
/// `RenderingServer.PRIMITIVE_TRIANGLES`.
const PRIMITIVE_TRIANGLES: i64 = 3;

/// The extra cull margin of a shadow receiver (`scene_lighting.gd`
/// `RECEIVER_EXTRA_CULL_MARGIN_M`), which a road chunk's node is.
const RECEIVER_EXTRA_CULL_MARGIN_M: f32 = 2.0;

/// The bounds every tool mesh and instance is given: vertices move in place without the
/// engine's bounds following, so the tool's drawing is never culled (as the nodes' `custom_aabb`
/// before round R2-3).
fn never_culled() -> Aabb {
    Aabb::new(
        Vector3::new(-1.0e6, -1.0e4, -1.0e6),
        Vector3::new(2.0e6, 2.0e4, 2.0e6),
    )
}

/// The `RenderingServer` drawing backend.
pub(crate) struct GodotBackend {
    server: Gd<RenderingServer>,
    scenario: Rid,
    // The materials, kept alive; the line material (none for the preview's chunks) also takes
    // the dash focus.
    _materials: Vec<Gd<Material>>,
    line: Option<Gd<ShaderMaterial>>,
    dash_focus: StringName,
    // Upload arrays by length: vertex buffers, attribute buffers, index buffers.
    vertices: Uploads,
    attributes: Uploads,
    indices: Uploads,
    // `mesh_add_surface`'s dictionary per surface kind (lines, points, fill) and its keys.
    surfaces: [Option<VarDictionary>; 3],
    keys: SurfaceKeys,
    // A road chunk layer's upload arrays, resized to each surface (the engine reallocates only
    // across a power of two), and its dictionary.
    road_vertices: PackedByteArray,
    road_attributes: PackedByteArray,
    road: Option<VarDictionary>,
    // A resident chunk's run write, resized to each run (round R2-4b).
    runs: PackedByteArray,
}

// `mesh_add_surface`'s keys, made once (a `&str` key is a new string each call).
struct SurfaceKeys {
    vertex_data: Variant,
    attribute_data: Variant,
    index_data: Variant,
    vertex_count: Variant,
    index_count: Variant,
    format: Variant,
    aabb: Variant,
}

// The upload arrays of one buffer kind by length, at most `UPLOADS_KEPT`, the oldest dropped: a
// write of a length met before reuses its array's storage.
#[derive(Default)]
struct Uploads {
    arrays: Vec<PackedByteArray>,
}

const UPLOADS_KEPT: usize = 8;

impl Uploads {
    // An array of `bytes`' length holding `bytes`. `O(bytes + UPLOADS_KEPT)`.
    fn filled(&mut self, bytes: &[u8]) -> &PackedByteArray {
        let index = match self.arrays.iter().position(|array| array.len() == bytes.len()) {
            Some(index) => index,
            None => {
                if self.arrays.len() == UPLOADS_KEPT {
                    self.arrays.remove(0);
                }
                let mut array = PackedByteArray::new();
                array.resize(bytes.len());
                self.arrays.push(array);
                self.arrays.len() - 1
            }
        };
        let array = &mut self.arrays[index];
        array.as_mut_slice().copy_from_slice(bytes);
        array
    }
}

impl GodotBackend {
    /// A backend drawing into `scenario` with `materials` (kept alive) and the line material
    /// `line`, whose `dash_focus` it sets.
    pub(crate) fn new(scenario: Rid, line: Gd<ShaderMaterial>, materials: Vec<Gd<Material>>) -> Self {
        Self::with_line(scenario, Some(line), materials)
    }

    /// A backend drawing into `scenario` (none: the instances draw nowhere until a caller
    /// copies their meshes, as tests do) with `materials` (kept alive) and no line material:
    /// the preview's chunks (round R2-4a).
    pub(crate) fn plain(scenario: Rid, materials: Vec<Gd<Material>>) -> Self {
        Self::with_line(scenario, None, materials)
    }

    fn with_line(
        scenario: Rid,
        line: Option<Gd<ShaderMaterial>>,
        materials: Vec<Gd<Material>>,
    ) -> Self {
        Self {
            server: RenderingServer::singleton(),
            scenario,
            _materials: materials,
            line,
            dash_focus: StringName::from("dash_focus"),
            vertices: Uploads::default(),
            attributes: Uploads::default(),
            indices: Uploads::default(),
            surfaces: [None, None, None],
            keys: SurfaceKeys {
                vertex_data: "vertex_data".to_variant(),
                attribute_data: "attribute_data".to_variant(),
                index_data: "index_data".to_variant(),
                vertex_count: "vertex_count".to_variant(),
                index_count: "index_count".to_variant(),
                format: "format".to_variant(),
                aabb: "aabb".to_variant(),
            },
            road_vertices: PackedByteArray::new(),
            road_attributes: PackedByteArray::new(),
            runs: PackedByteArray::new(),
            road: None,
        }
    }
}

// `array` resized to `bytes` and holding them. `O(bytes)`.
fn fill(array: &mut PackedByteArray, bytes: &[u8]) {
    array.resize(bytes.len());
    array.as_mut_slice().copy_from_slice(bytes);
}

// A surface kind's dictionary with its constant entries.
fn surface_dictionary(kind: SurfaceKind) -> VarDictionary {
    let mut dict = VarDictionary::new();
    dict.set("primitive", if kind.points() { PRIMITIVE_POINTS } else { PRIMITIVE_TRIANGLES });
    dict.set("format", kind.format() as i64);
    dict.set("aabb", never_culled());
    dict.set("uv_scale", Vector4::ZERO);
    dict
}

impl DrawBackend for GodotBackend {
    fn mesh_create(&mut self) -> Handle {
        self.server.mesh_create().to_u64()
    }

    fn instance_create(&mut self, mesh: Handle) -> Handle {
        let instance = self.server.instance_create2(Rid::new(mesh), self.scenario);
        self.server
            .instance_geometry_set_cast_shadows_setting(instance, ShadowCastingSetting::OFF);
        self.server.instance_set_custom_aabb(instance, never_culled());
        self.server.instance_set_visible(instance, false);
        instance.to_u64()
    }

    fn mesh_add_surface(&mut self, mesh: Handle, surface: &Surface) {
        let keys = &self.keys;
        let dict = self.surfaces[surface.kind as usize]
            .get_or_insert_with(|| surface_dictionary(surface.kind));
        dict.set(keys.vertex_data.clone(), self.vertices.filled(surface.vertices).clone());
        dict.set(keys.vertex_count.clone(), i64::from(surface.vertex_count));
        if surface.kind != SurfaceKind::Fill {
            dict.set(keys.attribute_data.clone(), self.attributes.filled(surface.attributes).clone());
        }
        if surface.kind == SurfaceKind::Lines {
            dict.set(keys.index_data.clone(), self.indices.filled(surface.indices).clone());
            dict.set(keys.index_count.clone(), i64::from(surface.index_count));
        }
        let mesh = Rid::new(mesh);
        let index = self.server.mesh_get_surface_count(mesh);
        self.server.mesh_add_surface(mesh, &*dict);
        self.server
            .mesh_surface_set_material(mesh, index, Rid::new(surface.material));
        // The arrays go back to the pools unshared, so the next write does not copy them.
        for key in [&keys.vertex_data, &keys.attribute_data, &keys.index_data] {
            if dict.contains_key(key.clone()) {
                dict.set(key.clone(), Variant::nil());
            }
        }
    }

    fn instance_create_plain(&mut self, mesh: Handle) -> Handle {
        let instance = self.server.instance_create2(Rid::new(mesh), self.scenario);
        self.server
            .instance_geometry_set_cast_shadows_setting(instance, ShadowCastingSetting::OFF);
        self.server
            .instance_set_extra_visibility_margin(instance, RECEIVER_EXTRA_CULL_MARGIN_M);
        self.server.instance_set_visible(instance, false);
        instance.to_u64()
    }

    fn mesh_add_road_surface(&mut self, mesh: Handle, surface: &RoadSurface, material: Handle) {
        let keys = &self.keys;
        let dict = self.road.get_or_insert_with(|| {
            let mut dict = VarDictionary::new();
            dict.set("primitive", PRIMITIVE_TRIANGLES);
            dict.set("uv_scale", Vector4::ZERO);
            dict
        });
        fill(&mut self.road_vertices, &surface.vertex_data);
        dict.set(keys.vertex_data.clone(), self.road_vertices.clone());
        if surface.attribute_data.is_empty() {
            dict.remove(keys.attribute_data.clone());
        } else {
            fill(&mut self.road_attributes, &surface.attribute_data);
            dict.set(keys.attribute_data.clone(), self.road_attributes.clone());
        }
        dict.set(keys.format.clone(), surface.format as i64);
        dict.set(keys.vertex_count.clone(), i64::from(surface.vertex_count));
        let [x, y, z, w, h, d] = surface.aabb;
        dict.set(
            keys.aabb.clone(),
            Aabb::new(Vector3::new(x, y, z), Vector3::new(w, h, d)),
        );
        let mesh = Rid::new(mesh);
        let index = self.server.mesh_get_surface_count(mesh);
        self.server.mesh_add_surface(mesh, &*dict);
        self.server
            .mesh_surface_set_material(mesh, index, Rid::new(material));
        // The arrays go back unshared, so the next fill does not copy them.
        for key in [&keys.vertex_data, &keys.attribute_data] {
            if dict.contains_key(key.clone()) {
                dict.set(key.clone(), Variant::nil());
            }
        }
    }

    fn instance_set_origin(&mut self, instance: Handle, origin: [f32; 3]) {
        let transform = Transform3D::new(Basis::IDENTITY, Vector3::from_array(origin));
        self.server
            .instance_set_transform(Rid::new(instance), transform);
    }

    fn mesh_clear(&mut self, mesh: Handle) {
        self.server.mesh_clear(Rid::new(mesh));
    }

    fn mesh_update_region(&mut self, mesh: Handle, surface: u32, region: Region, bytes: &[u8]) {
        // The engine copies the bytes into the GPU buffer during the call.
        let (mesh, surface) = (Rid::new(mesh), surface as i32);
        match region {
            Region::Vertices => {
                let upload = self.vertices.filled(bytes);
                self.server.mesh_surface_update_vertex_region(mesh, surface, 0, upload)
            }
            Region::Attributes => {
                let upload = self.attributes.filled(bytes);
                self.server.mesh_surface_update_attribute_region(mesh, surface, 0, upload)
            }
        }
    }

    fn mesh_set_material(&mut self, mesh: Handle, surface: u32, material: Handle) {
        self.server
            .mesh_surface_set_material(Rid::new(mesh), surface as i32, Rid::new(material));
    }

    fn instance_set_visible(&mut self, instance: Handle, visible: bool) {
        self.server.instance_set_visible(Rid::new(instance), visible);
    }

    fn mesh_write_vertices(&mut self, mesh: Handle, surface: u32, offset: u32, bytes: &[u8]) {
        // The engine copies the bytes into the GPU buffer during the call.
        fill(&mut self.runs, bytes);
        self.server.mesh_surface_update_vertex_region(
            Rid::new(mesh),
            surface as i32,
            offset as i32,
            &self.runs,
        );
    }

    fn set_dash_focus(&mut self, focus: [f32; 3]) {
        if let Some(line) = &mut self.line {
            let value: Variant = Vector3::from_array(focus).to_variant();
            line.set_shader_parameter(&self.dash_focus, &value);
        }
    }

    fn free(&mut self, handle: Handle) {
        self.server.free_rid(Rid::new(handle));
    }
}
