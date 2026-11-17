// SPDX-License-Identifier: GPL-2.0-only

//! The recorder backend (tests, `ROAD-112` round R2-3): records every drawing command and keeps
//! the state they leave (meshes' surfaces and bytes, instances' visibility), so the draw module
//! is tested without Godot.

use super::backend::{DrawBackend, Handle, Region, Surface, SurfaceKind};
use crate::nodes::sim::render::stream::layers::roads::surfaces::RoadSurface;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// A recorded command.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Command {
    MeshCreate(Handle),
    InstanceCreate(Handle, Handle),
    AddSurface(Handle, SurfaceKind, u32),
    AddRoadSurface(Handle, u64, u32, Handle),
    Origin(Handle, [f32; 3]),
    Clear(Handle),
    Update(Handle, u32, Region, usize),
    Material(Handle, u32, Handle),
    Visible(Handle, bool),
    Focus([f32; 3]),
    Free(Handle),
}

/// A recorded surface: its kind, material and bytes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecordedSurface {
    pub(crate) kind: SurfaceKind,
    pub(crate) material: Handle,
    pub(crate) vertices: Vec<u8>,
    pub(crate) attributes: Vec<u8>,
    pub(crate) indices: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Recorder {
    next: Handle,
    pub(crate) commands: Vec<Command>,
    pub(crate) meshes: BTreeMap<Handle, Vec<RecordedSurface>>,
    pub(crate) instances: BTreeMap<Handle, (Handle, bool)>,
    // Road chunk layers' surfaces by mesh: the encoded surface and its material.
    pub(crate) road_meshes: BTreeMap<Handle, Vec<(RoadSurface, Handle)>>,
    // Objects made and not freed, shared so a test reads it after the drawing dropped.
    pub(crate) live: Rc<Cell<i64>>,
}

impl Recorder {
    fn make(&mut self) -> Handle {
        self.next += 1;
        self.live.set(self.live.get() + 1);
        self.next
    }

    // Whether `mesh`'s instance is shown.
    pub(crate) fn shown(&self, mesh: Handle) -> bool {
        self.instances
            .values()
            .any(|&(base, visible)| base == mesh && visible)
    }
}

impl DrawBackend for Recorder {
    fn mesh_create(&mut self) -> Handle {
        let mesh = self.make();
        self.meshes.insert(mesh, Vec::new());
        self.commands.push(Command::MeshCreate(mesh));
        mesh
    }

    fn instance_create(&mut self, mesh: Handle) -> Handle {
        let instance = self.make();
        self.instances.insert(instance, (mesh, false));
        self.commands.push(Command::InstanceCreate(instance, mesh));
        instance
    }

    fn mesh_add_surface(&mut self, mesh: Handle, surface: &Surface) {
        let stride = match surface.kind {
            SurfaceKind::Lines => 20,
            SurfaceKind::Points => 8,
            SurfaceKind::Fill => 0,
        };
        assert_eq!(surface.vertices.len(), surface.vertex_count as usize * 12);
        assert_eq!(surface.attributes.len(), surface.vertex_count as usize * stride);
        self.commands
            .push(Command::AddSurface(mesh, surface.kind, surface.vertex_count));
        self.meshes.get_mut(&mesh).expect("a live mesh").push(RecordedSurface {
            kind: surface.kind,
            material: surface.material,
            vertices: surface.vertices.to_vec(),
            attributes: surface.attributes.to_vec(),
            indices: surface.indices.to_vec(),
        });
    }

    fn instance_create_plain(&mut self, mesh: Handle) -> Handle {
        self.instance_create(mesh)
    }

    fn mesh_add_road_surface(&mut self, mesh: Handle, surface: &RoadSurface, material: Handle) {
        assert!(self.meshes.contains_key(&mesh), "a live mesh");
        self.commands.push(Command::AddRoadSurface(
            mesh,
            surface.format,
            surface.vertex_count,
            material,
        ));
        self.road_meshes
            .entry(mesh)
            .or_default()
            .push((surface.clone(), material));
    }

    fn instance_set_origin(&mut self, instance: Handle, origin: [f32; 3]) {
        assert!(self.instances.contains_key(&instance), "a live instance");
        self.commands.push(Command::Origin(instance, origin));
    }

    fn mesh_clear(&mut self, mesh: Handle) {
        self.commands.push(Command::Clear(mesh));
        self.meshes.get_mut(&mesh).expect("a live mesh").clear();
        self.road_meshes.remove(&mesh);
    }

    fn mesh_update_region(&mut self, mesh: Handle, surface: u32, region: Region, bytes: &[u8]) {
        self.commands
            .push(Command::Update(mesh, surface, region, bytes.len()));
        let target = &mut self.meshes.get_mut(&mesh).expect("a live mesh")[surface as usize];
        let buffer = match region {
            Region::Vertices => &mut target.vertices,
            Region::Attributes => &mut target.attributes,
        };
        assert!(bytes.len() <= buffer.len(), "an in-place write fits its surface");
        buffer[..bytes.len()].copy_from_slice(bytes);
    }

    fn mesh_set_material(&mut self, mesh: Handle, surface: u32, material: Handle) {
        self.commands.push(Command::Material(mesh, surface, material));
        self.meshes.get_mut(&mesh).expect("a live mesh")[surface as usize].material = material;
    }

    fn instance_set_visible(&mut self, instance: Handle, visible: bool) {
        self.commands.push(Command::Visible(instance, visible));
        self.instances.get_mut(&instance).expect("a live instance").1 = visible;
    }

    fn set_dash_focus(&mut self, focus: [f32; 3]) {
        self.commands.push(Command::Focus(focus));
    }

    fn free(&mut self, handle: Handle) {
        self.commands.push(Command::Free(handle));
        self.road_meshes.remove(&handle);
        let mesh = self.meshes.remove(&handle).is_some();
        let instance = self.instances.remove(&handle).is_some();
        assert!(mesh || instance, "frees a live object once");
        self.live.set(self.live.get() - 1);
    }
}
