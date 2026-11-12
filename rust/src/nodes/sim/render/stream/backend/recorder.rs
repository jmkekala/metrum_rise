// SPDX-License-Identifier: GPL-2.0-only

//! The recording backend (tests): every command in order, the state the commands leave on each
//! object (what an instance would draw), and a digest of the sequence, so the layers' per-frame
//! invariants are checked without Godot.

use super::{Handle, RenderBackend, Scenario, Shadow, Vector};
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use std::collections::BTreeMap;

/// One backend call, buffers by digest.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Command {
    MultimeshCreate(Handle, u32, bool, bool),
    MultimeshReallocate(Handle, u32),
    MultimeshMesh(Handle, u32),
    MultimeshBuffer(Handle, u64, usize, u32),
    MultimeshVisible(Handle, u32),
    MultimeshAabb(Handle, Aabb),
    InstanceCreate(Handle, Handle, Scenario),
    InstanceOrigin(Handle, [f32; 3]),
    InstanceVisible(Handle, bool),
    InstanceShadow(Handle, Shadow),
    InstanceRange(Handle, [f32; 2]),
    InstanceAabb(Handle, Aabb),
    InstanceMaterial(Handle, Option<u32>),
    InstanceParameter(Handle, u32, f32),
    Free(Handle),
    MaterialCreate(Handle, Handle),
    MaterialTexture(Handle, u32, Option<Handle>),
    InstanceMaterialHandle(Handle, Option<Handle>),
    InstanceVector(Handle, u32, Vector),
    TextureLayer(Handle, u32, [u32; 2], u64),
    TextureCopy(Handle, Handle, u32),
    TextureForget(Handle),
}

/// A recorded material: its shader and its texture parameters.
#[derive(Clone, Debug, Default)]
pub(crate) struct RecordedMaterial {
    pub(crate) shader: Handle,
    pub(crate) textures: BTreeMap<u32, Handle>,
}

/// A recorded `MultiMesh`.
#[derive(Clone, Debug, Default)]
pub(crate) struct RecordedMultimesh {
    pub(crate) capacity: u32,
    pub(crate) colors: bool,
    pub(crate) custom: bool,
    pub(crate) mesh: Option<u32>,
    // `None`: every instance draws.
    pub(crate) visible_count: Option<u32>,
    pub(crate) aabb: Aabb,
    // The batch's own rows (the padding is checked when written).
    pub(crate) rows: Vec<f32>,
}

/// A recorded instance.
#[derive(Clone, Debug)]
pub(crate) struct RecordedInstance {
    pub(crate) base: Handle,
    pub(crate) scenario: Scenario,
    pub(crate) visible: bool,
    pub(crate) shadow: Shadow,
    pub(crate) origin: [f32; 3],
    pub(crate) range: [f32; 2],
    pub(crate) aabb: Aabb,
    pub(crate) material: Option<u32>,
    pub(crate) parameters: Vec<(u32, f32)>,
    // A material handle override (`instance_set_material_handle`) and vector parameters.
    pub(crate) material_handle: Option<Handle>,
    pub(crate) vectors: BTreeMap<u32, Vector>,
}

/// Records commands and keeps the state they leave.
#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) commands: Vec<Command>,
    pub(crate) multimeshes: BTreeMap<Handle, RecordedMultimesh>,
    pub(crate) instances: BTreeMap<Handle, RecordedInstance>,
    pub(crate) materials: BTreeMap<Handle, RecordedMaterial>,
    // Every written texture layer's bytes, by texture and layer.
    pub(crate) textures: BTreeMap<Handle, BTreeMap<u32, Vec<u8>>>,
    upload: Vec<u8>,
    // Whether the reflection scenario exists.
    pub(crate) reflection: bool,
    next: Handle,
}

impl Recorder {
    /// A recorder with both scenarios.
    pub(crate) fn new() -> Self {
        Self {
            reflection: true,
            next: 1,
            ..Self::default()
        }
    }

    /// A digest of every command so far: FNV-1a over their debug forms, stable across builds.
    pub(crate) fn digest(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for command in &self.commands {
            for byte in format!("{command:?};").bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        hash
    }

    /// The instances that draw in `scenario`.
    pub(crate) fn visible(
        &self,
        scenario: Scenario,
    ) -> impl Iterator<Item = (&Handle, &RecordedInstance)> {
        self.instances
            .iter()
            .filter(move |(_, instance)| instance.visible && instance.scenario == scenario)
    }

    fn new_handle(&mut self) -> Handle {
        let handle = self.next;
        self.next += 1;
        handle
    }

    fn instance(&mut self, handle: Handle) -> &mut RecordedInstance {
        self.instances.get_mut(&handle).expect("live instance")
    }
}

fn digest(rows: &[f32]) -> u64 {
    rows.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, value| {
        value
            .to_bits()
            .to_le_bytes()
            .iter()
            .fold(hash, |hash, &byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            })
    })
}

impl RenderBackend for Recorder {
    fn multimesh_create(&mut self, capacity: u32, colors: bool, custom: bool) -> Handle {
        let handle = self.new_handle();
        self.commands
            .push(Command::MultimeshCreate(handle, capacity, colors, custom));
        self.multimeshes.insert(
            handle,
            RecordedMultimesh {
                capacity,
                colors,
                custom,
                ..RecordedMultimesh::default()
            },
        );
        handle
    }

    fn multimesh_reallocate(
        &mut self,
        multimesh: Handle,
        capacity: u32,
        _colors: bool,
        _custom: bool,
    ) {
        self.commands
            .push(Command::MultimeshReallocate(multimesh, capacity));
        let recorded = self
            .multimeshes
            .get_mut(&multimesh)
            .expect("live multimesh");
        // As the engine: the visible count clamps through the empty allocation.
        recorded.visible_count = recorded.visible_count.map(|_| 0);
        recorded.rows.clear();
    }

    fn multimesh_set_mesh(&mut self, multimesh: Handle, mesh: u32) {
        self.commands.push(Command::MultimeshMesh(multimesh, mesh));
        self.multimeshes
            .get_mut(&multimesh)
            .expect("live multimesh")
            .mesh = Some(mesh);
    }

    fn multimesh_set_buffer(
        &mut self,
        multimesh: Handle,
        rows: &[f32],
        stride: usize,
        capacity: u32,
    ) {
        let recorded = self
            .multimeshes
            .get_mut(&multimesh)
            .expect("live multimesh");
        assert_eq!(recorded.capacity, capacity, "a buffer fills its multimesh");
        let lanes = 12 + 4 * usize::from(recorded.colors) + 4 * usize::from(recorded.custom);
        assert_eq!(
            stride, lanes,
            "the buffer's stride matches the multimesh's format"
        );
        assert!(rows.len().is_multiple_of(stride) && rows.len() / stride <= capacity as usize);
        assert!(!rows.is_empty(), "the padding copies a first instance");
        recorded.rows.clear();
        recorded.rows.extend_from_slice(rows);
        self.commands.push(Command::MultimeshBuffer(
            multimesh,
            digest(rows),
            stride,
            capacity,
        ));
    }

    fn multimesh_set_visible_instances(&mut self, multimesh: Handle, count: u32) {
        self.commands
            .push(Command::MultimeshVisible(multimesh, count));
        self.multimeshes
            .get_mut(&multimesh)
            .expect("live multimesh")
            .visible_count = Some(count);
    }

    fn multimesh_set_custom_aabb(&mut self, multimesh: Handle, aabb: Aabb) {
        self.commands.push(Command::MultimeshAabb(multimesh, aabb));
        self.multimeshes
            .get_mut(&multimesh)
            .expect("live multimesh")
            .aabb = aabb;
    }

    fn instance_create(&mut self, base: Handle, scenario: Scenario) -> Option<Handle> {
        if scenario == Scenario::Reflection && !self.reflection {
            return None;
        }
        assert!(
            self.multimeshes.contains_key(&base),
            "an instance of a live multimesh"
        );
        let handle = self.new_handle();
        self.commands
            .push(Command::InstanceCreate(handle, base, scenario));
        self.instances.insert(
            handle,
            RecordedInstance {
                base,
                scenario,
                // The engine's defaults.
                visible: true,
                shadow: Shadow::On,
                origin: [0.0; 3],
                range: [0.0; 2],
                aabb: Aabb::default(),
                material: None,
                parameters: Vec::new(),
                material_handle: None,
                vectors: BTreeMap::new(),
            },
        );
        Some(handle)
    }

    fn instance_set_origin(&mut self, instance: Handle, origin: [f32; 3]) {
        self.commands
            .push(Command::InstanceOrigin(instance, origin));
        self.instance(instance).origin = origin;
    }

    fn instance_set_visible(&mut self, instance: Handle, visible: bool) {
        self.commands
            .push(Command::InstanceVisible(instance, visible));
        self.instance(instance).visible = visible;
    }

    fn instance_set_shadow(&mut self, instance: Handle, shadow: Shadow) {
        self.commands
            .push(Command::InstanceShadow(instance, shadow));
        self.instance(instance).shadow = shadow;
    }

    fn instance_set_range(&mut self, instance: Handle, range: [f32; 2]) {
        self.commands.push(Command::InstanceRange(instance, range));
        self.instance(instance).range = range;
    }

    fn instance_set_custom_aabb(&mut self, instance: Handle, aabb: Aabb) {
        self.commands.push(Command::InstanceAabb(instance, aabb));
        self.instance(instance).aabb = aabb;
    }

    fn instance_set_material(&mut self, instance: Handle, material: Option<u32>) {
        self.commands
            .push(Command::InstanceMaterial(instance, material));
        self.instance(instance).material = material;
    }

    fn instance_set_parameter(&mut self, instance: Handle, parameter: u32, value: f32) {
        self.commands
            .push(Command::InstanceParameter(instance, parameter, value));
        self.instance(instance).parameters.push((parameter, value));
    }

    fn free(&mut self, handle: Handle) {
        self.commands.push(Command::Free(handle));
        if self.materials.remove(&handle).is_some() {
            return;
        }
        if self.instances.remove(&handle).is_none() {
            assert!(
                self.multimeshes.remove(&handle).is_some(),
                "free of a live object"
            );
            assert!(
                self.instances
                    .values()
                    .all(|instance| instance.base != handle),
                "a multimesh outlives its instances"
            );
        }
    }
    fn material_create(&mut self, shader: Handle) -> Handle {
        let handle = self.new_handle();
        self.commands.push(Command::MaterialCreate(handle, shader));
        self.materials.insert(
            handle,
            RecordedMaterial {
                shader,
                textures: BTreeMap::new(),
            },
        );
        handle
    }

    fn material_set_texture(&mut self, material: Handle, parameter: u32, texture: Option<Handle>) {
        self.commands
            .push(Command::MaterialTexture(material, parameter, texture));
        let textures = &mut self
            .materials
            .get_mut(&material)
            .expect("live material")
            .textures;
        match texture {
            Some(texture) => textures.insert(parameter, texture),
            None => textures.remove(&parameter),
        };
    }

    fn instance_set_material_handle(&mut self, instance: Handle, material: Option<Handle>) {
        self.commands
            .push(Command::InstanceMaterialHandle(instance, material));
        self.instance(instance).material_handle = material;
    }

    fn instance_set_vector(&mut self, instance: Handle, parameter: u32, value: Vector) {
        self.commands
            .push(Command::InstanceVector(instance, parameter, value));
        self.instance(instance).vectors.insert(parameter, value);
    }

    fn texture_write_layer(
        &mut self,
        texture: Handle,
        layer: u32,
        size: [u32; 2],
        fill: &mut dyn FnMut(&mut [u8]),
    ) {
        self.upload.clear();
        self.upload.resize(size[0] as usize * size[1] as usize * 16, 0);
        fill(&mut self.upload);
        self.commands
            .push(Command::TextureLayer(texture, layer, size, bytes_digest(&self.upload)));
        self.textures
            .entry(texture)
            .or_default()
            .insert(layer, self.upload.clone());
    }

    fn texture_copy_layers(&mut self, from: Handle, to: Handle, _size: [u32; 2], layers: u32) {
        self.commands.push(Command::TextureCopy(from, to, layers));
        let copied: Vec<(u32, Vec<u8>)> = self
            .textures
            .get(&from)
            .map(|written| written.range(..layers).map(|(&layer, bytes)| (layer, bytes.clone())).collect())
            .unwrap_or_default();
        self.textures.entry(to).or_default().extend(copied);
    }

    fn texture_forget(&mut self, texture: Handle) {
        self.commands.push(Command::TextureForget(texture));
    }
}

// FNV-1a over `bytes`.
fn bytes_digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
