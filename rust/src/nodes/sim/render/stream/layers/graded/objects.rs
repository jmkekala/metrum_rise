// SPDX-License-Identifier: GPL-2.0-only

//! The graded cell sets' `RenderingServer` objects (`RENDER-32-K`, G2c-1): what
//! `terrain_graded_tiles.gd` made of the book's cell effects 30-46 with nodes, made through the
//! stream backend. Per set one `MultiMesh` and instance per cell mesh (the base cells, `GL0`-`GL3`
//! and the behind-the-road variant's four), one material per graded mesh (made here from the
//! graded shaders, binding its block's page and carrying its patch's synced parameters), and a
//! wall `MultiMesh` drawn with the wall material. A set draws where its patch node stood: its
//! instances stand at the patch's origin, and an instance draws while its set draws its patch and
//! its mesh has instances, as the set's root and node visibility did.
//!
//! The calls keep the GDScript's order and its rules: a mesh is set before any buffer and a base
//! mesh change on a written buffer drops the buffer first (`TERRAIN-10`); a set binds its
//! materials and instance uniforms only while it draws (Godot reserves instance uniform slots for
//! an instance whose material declares them); every mesh's buffer is published whole from the
//! book's CPU buffer. The material parameter sync reads the patch's material through the
//! `RenderingServer` and is the book's (`book.rs`): it needs Godot's values.
//!
//! Bounds: an effect `O(1)` but a publication (`O(the patch's tile places)`), a wall rewrite
//! (`O(capacity)`) and a show or hide (`O(the set's meshes)`). No allocation once the sets exist
//! but a set made past the prewarm and a set's first variant (the build path, as before) and a
//! wall buffer's growth.

use super::cells::{
    CELL_BASE, CELL_BEHIND, CELL_MESHES, CELL_PLAIN, MAX_WALLS_PER_PATCH, Patch,
    WALL_INSTANCE_FLOATS, base_code,
};
use super::ordered::OrderedMap;
use super::state::{Effect, PatchKey};
use crate::nodes::sim::render::stream::backend::{Handle, RenderBackend, Scenario, Shadow, Vector};
use crate::nodes::sim::render::vegetation_product::godot_math::Aabb;
use std::collections::HashMap;

// Floats per cell instance (transform rows, then custom data) and per wall instance.
const CELL_STRIDE: usize = 16;
const WALL_STRIDE: usize = 12;

/// The catalogue entries and handles the cells draw with.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CellCatalogue {
    /// The mesh index of each lattice code (0-6).
    pub lattice: [u32; 7],
    /// The wall quad's mesh index.
    pub wall: u32,
    /// Parameter indices: `cell_mode`, `cell_patch_rect_hm`, `cell_lattice_anchor_hm`,
    /// `graded_texels`, `graded_atlas`.
    pub mode: u32,
    pub rect: u32,
    pub anchor: u32,
    pub texels: u32,
    pub atlas: u32,
    /// The graded shader, its behind-the-road variant, and the wall material.
    pub graded_shader: Handle,
    pub behind_shader: Handle,
    pub wall_material: Handle,
    /// The base lattice's anchor in half-metres.
    pub anchor_hm: [i32; 2],
    /// Triangles of each lattice code's mesh (the perf channel's count).
    pub triangles: [i64; 7],
}

/// What a resident patch's set draws with, from `terrain.gd`'s patch entry: the patch node (its
/// instance id), its material and its culling box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PatchDraw {
    pub node: i64,
    pub material: Handle,
    pub aabb: Aabb,
}

/// What the book does after an effect: its trace line, a material sync, a counter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Applied {
    None,
    /// Set `set` of patch `key` published mesh `mesh` from float `at` of the published buffers.
    Published {
        key: PatchKey,
        mesh: u8,
        at: u32,
    },
    /// Patch `key`'s set's walls: `count` faces from float `start`, the `MultiMesh`'s
    /// instances after it grew, the faces past them.
    Walls {
        key: PatchKey,
        start: u32,
        count: u32,
        instances: u32,
        truncated: u32,
    },
    /// A set stopped drawing.
    Hidden,
    /// Set `set` started drawing patch `key`: its materials sync where the watched values moved.
    Shown {
        set: u32,
        key: PatchKey,
    },
    /// Set `set` took the variant: its new materials take the set's synced values.
    Variant {
        set: u32,
    },
    /// Drawing set `set` was rewritten whole: its materials sync with patch `key`'s.
    Rebuilt {
        set: u32,
        key: PatchKey,
    },
    /// Set `set` began a build: its material sync starts over.
    Begun {
        set: u32,
    },
    /// A set build completed.
    Built,
}

// One cell mesh (or the walls) of a set: its objects and what they were last given.
#[derive(Clone, Debug, Default)]
struct Part {
    multimesh: Handle,
    instance: Handle,
    material: Option<Handle>,
    mesh: Option<u32>,
    // The visible instance count as last sent (`None`: unknown since an allocation).
    count: Option<u32>,
    // The node's own visibility (it has instances), whether the instance draws, the material
    // override it holds.
    has: bool,
    drawn: bool,
    bound: Option<Handle>,
    // The textures its material binds (texels, atlas), as sent.
    textures: [Option<Handle>; 2],
}

/// One set's objects.
#[derive(Clone, Debug)]
pub(crate) struct SetObjects {
    parts: Vec<Part>,
    walls: Part,
    wall_capacity: u32,
    // The root's visibility (it draws its patch) and the patch node it stands under (0: none).
    shown: bool,
    parent: i64,
    origin: Option<[f32; 3]>,
    aabb: Aabb,
    // A copy of the cells' `hidden`, so a set's visibility needs only the set.
    hidden: bool,
    // The patch it was last taken or begun for (`cells["key"]`, the trace's).
    key: PatchKey,
    rect_hm: [i32; 4],
    step: i32,
    /// The material sync: done for this build, and where a budgeted sync goes on.
    pub synced: bool,
    pub sync_next: usize,
}

/// Every set's objects, by the book's set id, and what they draw with.
#[derive(Debug, Default)]
pub(crate) struct CellObjects {
    pub(crate) sets: Vec<SetObjects>,
    catalogue: CellCatalogue,
    per_patch: u32,
    pub(crate) patches: HashMap<PatchKey, PatchDraw>,
    // Per page id its (texels, atlas) textures.
    pub(crate) pages: HashMap<i64, [Handle; 2]>,
    // A wall buffer, reused.
    wall_rows: Vec<f32>,
    /// Objects made (`multimeshes_created`, `materials_created`) and variants given.
    pub(crate) multimeshes_created: u64,
    pub(crate) materials_created: u64,
    pub(crate) variants_created: u64,
    // Whether the cells draw at all (the terrain node's visibility).
    hidden: bool,
}

// `_cell_mesh_for`: the lattice code of cell mesh `index` in a set of `step`. GL3 cells take the
// finest GL3 mesh (step 2); only the base cells follow the step.
fn cell_mesh_code(index: usize, step: i32) -> usize {
    let level = if index >= CELL_BEHIND {
        index - CELL_BEHIND
    } else {
        index
    };
    let code = if level < 3 {
        level as u8
    } else if index == CELL_BASE {
        base_code(step)
    } else {
        base_code(2)
    };
    usize::from(code.min(6))
}

impl CellObjects {
    /// Cells of `per_patch` tile places drawn with `catalogue`.
    pub(crate) fn new(catalogue: CellCatalogue, per_patch: u32) -> Self {
        Self {
            catalogue,
            per_patch,
            ..Self::default()
        }
    }

    /// Makes the objects of sets up to `count` (setup's prewarm, or a set past it).
    pub(crate) fn ensure(&mut self, backend: &mut impl RenderBackend, count: usize) {
        while self.sets.len() < count {
            let mut parts = Vec::with_capacity(CELL_MESHES);
            for index in 0..CELL_PLAIN {
                parts.push(self.new_part(backend, index));
            }
            let walls = self.new_walls(backend);
            self.sets.push(SetObjects {
                parts,
                walls,
                wall_capacity: MAX_WALLS_PER_PATCH,
                shown: false,
                parent: 0,
                origin: None,
                aabb: Aabb::default(),
                hidden: self.hidden,
                key: (0, 0),
                rect_hm: [0; 4],
                step: -1,
                synced: false,
                sync_next: 0,
            });
        }
    }

    // `_add_cell_mesh`: a cell mesh's `MultiMesh` of the patch's tile places, no instance drawn,
    // its mesh before any buffer (but the base cells', set at the set's first build), its
    // instance hidden and casting no shadow, its graded material.
    fn new_part(&mut self, backend: &mut impl RenderBackend, index: usize) -> Part {
        let multimesh = backend.multimesh_create(self.per_patch, false, true);
        backend.multimesh_set_visible_instances(multimesh, 0);
        let mesh = (index != CELL_BASE).then(|| self.catalogue.lattice[cell_mesh_code(index, 2)]);
        if let Some(mesh) = mesh {
            backend.multimesh_set_mesh(multimesh, mesh);
        }
        self.multimeshes_created += 1;
        let instance = backend
            .instance_create(multimesh, Scenario::Main)
            .expect("the main scenario is registered");
        backend.instance_set_visible(instance, false);
        backend.instance_set_shadow(instance, Shadow::Off);
        let material = (index != CELL_BASE).then(|| {
            self.materials_created += 1;
            let shader = if index >= CELL_BEHIND {
                self.catalogue.behind_shader
            } else {
                self.catalogue.graded_shader
            };
            backend.material_create(shader)
        });
        Part {
            multimesh,
            instance,
            material,
            mesh,
            count: Some(0),
            ..Part::default()
        }
    }

    // The wall `MultiMesh` (the quad, `MAX_WALLS_PER_PATCH` instances, none drawn) and its
    // instance, which keeps the wall material and casts shadows (a node's defaults).
    fn new_walls(&mut self, backend: &mut impl RenderBackend) -> Part {
        let multimesh = backend.multimesh_create(MAX_WALLS_PER_PATCH, false, false);
        backend.multimesh_set_mesh(multimesh, self.catalogue.wall);
        backend.multimesh_set_visible_instances(multimesh, 0);
        self.multimeshes_created += 1;
        let instance = backend
            .instance_create(multimesh, Scenario::Main)
            .expect("the main scenario is registered");
        backend.instance_set_visible(instance, false);
        backend.instance_set_material_handle(instance, Some(self.catalogue.wall_material));
        Part {
            multimesh,
            instance,
            mesh: Some(self.catalogue.wall),
            count: Some(0),
            bound: Some(self.catalogue.wall_material),
            ..Part::default()
        }
    }

    /// Whether the cells draw at all (the terrain node's visibility). O(sets) on a change.
    pub(crate) fn set_drawn(&mut self, backend: &mut impl RenderBackend, drawn: bool) {
        if self.hidden != drawn {
            return;
        }
        self.hidden = !drawn;
        for objects in &mut self.sets {
            objects.hidden = !drawn;
            Self::update_drawn(backend, objects);
        }
    }

    /// Frees every object (the renderer's disposal).
    pub(crate) fn free_all(&mut self, backend: &mut impl RenderBackend) {
        for set in self.sets.drain(..) {
            for part in set.parts.iter().chain([&set.walls]) {
                backend.free(part.instance);
                backend.free(part.multimesh);
                if let Some(material) = part.material {
                    backend.free(material);
                }
            }
        }
    }

    /// Applies one of the book's cell effects (30-39, 41-46) to its set's objects. `resident` is
    /// the book's resident patches, `published` and `walls` the floats the effects name.
    pub(crate) fn apply(
        &mut self,
        backend: &mut impl RenderBackend,
        effect: &Effect,
        resident: &OrderedMap<PatchKey, Patch>,
        published: &[f32],
        walls: &[f32],
    ) -> Applied {
        match *effect {
            Effect::CellAcquired { set, key } => {
                self.ensure(backend, set as usize + 1);
                let aabb = self
                    .patches
                    .get(&key)
                    .map(|patch| patch.aabb)
                    .unwrap_or_default();
                let objects = &mut self.sets[set as usize];
                objects.key = key;
                objects.aabb = aabb;
                for part in objects.parts.iter().chain([&objects.walls]) {
                    backend.multimesh_set_custom_aabb(part.multimesh, aabb);
                    backend.instance_set_custom_aabb(part.instance, aabb);
                }
            }
            Effect::CellPages { set, code, page } => {
                self.bind_level(backend, set, usize::from(code), page)
            }
            Effect::CellPark { set, key } => {
                let parent =
                    key.map(|key| (key, self.patches.get(&key).map_or(0, |patch| patch.node)));
                let objects = &mut self.sets[set as usize];
                match parent {
                    Some((key, node)) => {
                        objects.parent = node;
                        let origin = resident.get(&key).map(patch_origin);
                        Self::set_origin(backend, objects, origin);
                    }
                    None => objects.parent = 0,
                }
                Self::update_drawn(backend, objects);
            }
            Effect::CellBegin { set, key, step } => {
                let objects = &mut self.sets[set as usize];
                objects.key = key;
                if let Some(patch) = resident.get(&key) {
                    objects.rect_hm = rect_hm(patch.rect);
                }
                objects.step = step;
                objects.synced = false;
                return Applied::Begun { set };
            }
            Effect::CellBaseMesh { set, code, reset } => {
                let mesh = self.catalogue.lattice[usize::from(code)];
                let per_patch = self.per_patch;
                let base = &mut self.sets[set as usize].parts[CELL_BASE];
                if reset {
                    // A fresh allocation has no buffer to read back (TERRAIN-10); the book
                    // publishes the set's CPU buffer whole.
                    backend.multimesh_reallocate(base.multimesh, per_patch, false, true);
                    base.count = None;
                }
                if base.mesh != Some(mesh) {
                    backend.multimesh_set_mesh(base.multimesh, mesh);
                    base.mesh = Some(mesh);
                }
            }
            Effect::CellCountsCleared { set } => {
                let objects = &mut self.sets[set as usize];
                for at in 0..objects.parts.len() {
                    Self::set_count(backend, objects, at, 0);
                }
            }
            Effect::CellMeshes { set, first } => self.set_meshes(backend, set, usize::from(first)),
            Effect::CellPublish { set, mesh, at } => {
                let length = self.per_patch as usize * CELL_STRIDE;
                let rows = &published[at as usize..at as usize + length];
                let part = &self.sets[set as usize].parts[usize::from(mesh)];
                backend.multimesh_set_buffer(part.multimesh, rows, CELL_STRIDE, self.per_patch);
                let key = self.key_of(set);
                return Applied::Published { key, mesh, at };
            }
            Effect::CellCount { set, mesh, count } => {
                Self::set_count(
                    backend,
                    &mut self.sets[set as usize],
                    usize::from(mesh),
                    count,
                );
            }
            Effect::CellWalls {
                set,
                start,
                count,
                capacity,
                truncated,
            } => {
                self.set_walls(backend, set, &walls[start as usize..], count, capacity);
                let key = self.key_of(set);
                let instances = self.sets[set as usize].wall_capacity;
                return Applied::Walls {
                    key,
                    start,
                    count,
                    instances,
                    truncated,
                };
            }
            Effect::CellShow { set, key } => {
                self.set_meshes(backend, set, 0);
                let material = self.patches.get(&key).map(|patch| patch.material);
                let node = self.patches.get(&key).map_or(0, |patch| patch.node);
                let origin = resident.get(&key).map(patch_origin);
                let objects = &mut self.sets[set as usize];
                Self::bind(backend, objects, true);
                Self::bind_part(backend, &mut objects.parts[CELL_BASE], material);
                objects.parent = node;
                Self::set_origin(backend, objects, origin);
                objects.shown = true;
                Self::update_drawn(backend, objects);
                return Applied::Shown { set, key };
            }
            Effect::CellHide { set } => {
                let objects = &mut self.sets[set as usize];
                objects.shown = false;
                Self::update_drawn(backend, objects);
                Self::bind(backend, objects, false);
                return Applied::Hidden;
            }
            Effect::CellVariant { set, pages, shown } => {
                for index in CELL_BEHIND..CELL_MESHES {
                    let part = self.new_part(backend, index);
                    let objects = &mut self.sets[set as usize];
                    backend.multimesh_set_custom_aabb(part.multimesh, objects.aabb);
                    backend.instance_set_custom_aabb(part.instance, objects.aabb);
                    if let Some(origin) = objects.origin {
                        backend.instance_set_origin(part.instance, origin);
                    }
                    objects.parts.push(part);
                }
                self.variants_created += 1;
                for index in CELL_BEHIND..CELL_MESHES {
                    self.bind_material(backend, set, index, pages[index - CELL_BEHIND]);
                }
                if shown {
                    let objects = &mut self.sets[set as usize];
                    for at in CELL_BEHIND..CELL_MESHES {
                        let material = objects.parts[at].material;
                        Self::bind_part(backend, &mut objects.parts[at], material);
                    }
                }
                return Applied::Variant { set };
            }
            Effect::CellRebuilt { set, key } => {
                let material = self.patches.get(&key).map(|patch| patch.material);
                Self::bind_part(
                    backend,
                    &mut self.sets[set as usize].parts[CELL_BASE],
                    material,
                );
                return Applied::Rebuilt { set, key };
            }
            Effect::CellBuilt { .. } => return Applied::Built,
            _ => unreachable!("not a cell effect: {effect:?}"),
        }
        Applied::None
    }

    fn key_of(&self, set: u32) -> PatchKey {
        self.sets[set as usize].key
    }

    // `_bind_level`: the set's materials of level `code` (and its variant's) bind `page`.
    fn bind_level(&mut self, backend: &mut impl RenderBackend, set: u32, code: usize, page: i64) {
        for index in [code, code + CELL_BEHIND] {
            if index < self.sets[set as usize].parts.len() {
                self.bind_material(backend, set, index, page);
            }
        }
    }

    fn bind_material(
        &mut self,
        backend: &mut impl RenderBackend,
        set: u32,
        index: usize,
        page: i64,
    ) {
        let Some(material) = self.sets[set as usize].parts[index].material else {
            return;
        };
        let textures = self.pages.get(&page).copied();
        let textures = [
            textures.map(|textures| textures[0]),
            textures.map(|textures| textures[1]),
        ];
        backend.material_set_texture(material, self.catalogue.texels, textures[0]);
        backend.material_set_texture(material, self.catalogue.atlas, textures[1]);
        self.sets[set as usize].parts[index].textures = textures;
    }

    /// Set `set`'s bound textures per mesh (texels, atlas), as sent (none at the base cells).
    pub(crate) fn textures(&self, set: u32) -> impl Iterator<Item = [Option<Handle>; 2]> + '_ {
        self.sets[set as usize]
            .parts
            .iter()
            .map(|part| part.textures)
    }

    // `_set_cell_meshes`: a drawing set's meshes and instance uniforms from mesh `first` on.
    fn set_meshes(&mut self, backend: &mut impl RenderBackend, set: u32, first: usize) {
        let catalogue = self.catalogue;
        let objects = &mut self.sets[set as usize];
        for index in first..objects.parts.len() {
            let mesh = catalogue.lattice[cell_mesh_code(index, objects.step)];
            let part = &mut objects.parts[index];
            if part.mesh != Some(mesh) {
                backend.multimesh_set_mesh(part.multimesh, mesh);
                part.mesh = Some(mesh);
            }
            backend.instance_set_vector(
                part.instance,
                catalogue.rect,
                Vector::Int4(objects.rect_hm),
            );
            backend.instance_set_vector(
                part.instance,
                catalogue.anchor,
                Vector::Int2(catalogue.anchor_hm),
            );
        }
        if first <= CELL_BASE {
            backend.instance_set_parameter(objects.parts[CELL_BASE].instance, catalogue.mode, 2.0);
        }
    }

    // `_bind_materials`: the graded meshes take their materials (or none); unbound, the base
    // cells drop the patch's too.
    fn bind(backend: &mut impl RenderBackend, objects: &mut SetObjects, bound: bool) {
        for (index, part) in objects.parts.iter_mut().enumerate() {
            if index != CELL_BASE {
                let material = if bound { part.material } else { None };
                Self::bind_part(backend, part, material);
            } else if !bound {
                Self::bind_part(backend, part, None);
            }
        }
    }

    fn bind_part(backend: &mut impl RenderBackend, part: &mut Part, material: Option<Handle>) {
        if part.bound != material {
            backend.instance_set_material_handle(part.instance, material);
            part.bound = material;
        }
    }

    fn set_origin(
        backend: &mut impl RenderBackend,
        objects: &mut SetObjects,
        origin: Option<[f32; 3]>,
    ) {
        let Some(origin) = origin else {
            return;
        };
        if objects.origin == Some(origin) {
            return;
        }
        objects.origin = Some(origin);
        for part in objects.parts.iter().chain([&objects.walls]) {
            backend.instance_set_origin(part.instance, origin);
        }
    }

    fn set_count(
        backend: &mut impl RenderBackend,
        objects: &mut SetObjects,
        at: usize,
        count: u32,
    ) {
        let part = &mut objects.parts[at];
        if part.count != Some(count) {
            backend.multimesh_set_visible_instances(part.multimesh, count);
            part.count = Some(count);
        }
        part.has = count > 0;
        let drawn = objects.shown && objects.parent != 0 && part.has && !objects.hidden;
        Self::draw_part(backend, part, drawn);
    }

    fn draw_part(backend: &mut impl RenderBackend, part: &mut Part, drawn: bool) {
        if part.drawn != drawn {
            backend.instance_set_visible(part.instance, drawn);
            part.drawn = drawn;
        }
    }

    // Each instance draws while its set draws (its root shown under a patch node) and its mesh
    // has instances.
    fn update_drawn(backend: &mut impl RenderBackend, objects: &mut SetObjects) {
        let root = objects.shown && objects.parent != 0 && !objects.hidden;
        for part in objects.parts.iter_mut().chain([&mut objects.walls]) {
            let drawn = root && part.has;
            Self::draw_part(backend, part, drawn);
        }
    }

    // `_set_walls`: `count` faces (three basis columns each) as the walls' instance transforms,
    // the `MultiMesh` grown to `capacity` first (growing drops what it holds). One buffer write
    // of the whole capacity, padded with copies of the first face, in place of a call per face.
    fn set_walls(
        &mut self,
        backend: &mut impl RenderBackend,
        set: u32,
        faces: &[f32],
        count: u32,
        capacity: u32,
    ) {
        let objects = &mut self.sets[set as usize];
        if capacity > objects.wall_capacity {
            backend.multimesh_reallocate(objects.walls.multimesh, capacity, false, false);
            objects.wall_capacity = capacity;
            objects.walls.count = None;
        }
        if count > 0 {
            let rows = &mut self.wall_rows;
            rows.clear();
            for face in faces[..count as usize * WALL_INSTANCE_FLOATS]
                .as_chunks::<WALL_INSTANCE_FLOATS>()
                .0
            {
                // Basis columns (x, y, z axes) as `TRANSFORM_3D` rows, origin zero.
                rows.extend_from_slice(&[
                    face[0], face[3], face[6], 0.0, face[1], face[4], face[7], 0.0, face[2],
                    face[5], face[8], 0.0,
                ]);
            }
            backend.multimesh_set_buffer(
                objects.walls.multimesh,
                rows,
                WALL_STRIDE,
                objects.wall_capacity,
            );
        }
        let walls = &mut objects.walls;
        if walls.count != Some(count) {
            backend.multimesh_set_visible_instances(walls.multimesh, count);
            walls.count = Some(count);
        }
        walls.has = count > 0;
        let drawn = objects.shown && objects.parent != 0 && walls.has && !objects.hidden;
        Self::draw_part(backend, walls, drawn);
    }

    /// A set's graded materials (with the meshes they serve), for the sync.
    pub(crate) fn materials(&self, set: u32) -> impl Iterator<Item = Handle> + '_ {
        self.sets[set as usize]
            .parts
            .iter()
            .filter_map(|part| part.material)
    }

    /// Set `set`'s view for tests, grass and the probe: per mesh its `MultiMesh`, the count last
    /// sent, whether its node has instances, whether its instance draws, its material override.
    pub(crate) fn parts(
        &self,
        set: u32,
    ) -> impl Iterator<Item = (Handle, u32, bool, bool, Option<Handle>)> + '_ {
        self.sets[set as usize].parts.iter().map(|part| {
            (
                part.multimesh,
                part.count.unwrap_or(0),
                part.has,
                part.drawn,
                part.bound,
            )
        })
    }

    /// `release_patch`'s part: set `set` returns to the pool, hidden, out of its patch node,
    /// unbound, every mesh and its walls drawing no instance. O(the set's meshes).
    pub(crate) fn release(&mut self, backend: &mut impl RenderBackend, set: u32) {
        let objects = &mut self.sets[set as usize];
        objects.shown = false;
        objects.parent = 0;
        Self::update_drawn(backend, objects);
        Self::bind(backend, objects, false);
        for part in objects.parts.iter_mut().chain([&mut objects.walls]) {
            part.has = false;
            if part.count != Some(0) {
                backend.multimesh_set_visible_instances(part.multimesh, 0);
                part.count = Some(0);
            }
        }
    }

    /// Adds what set `set` submits while it draws: instances per cell mesh into `out[..9]`,
    /// triangles into `out[9..]`.
    pub(crate) fn submitted(&self, set: u32, out: &mut [i64; 18]) {
        let objects = &self.sets[set as usize];
        if !objects.shown {
            return;
        }
        for (index, part) in objects.parts.iter().enumerate() {
            if !part.has {
                continue;
            }
            let count = i64::from(part.count.unwrap_or(0));
            out[index] += count;
            out[CELL_MESHES + index] +=
                count * self.catalogue.triangles[cell_mesh_code(index, objects.step)];
        }
    }

    /// Every set's cell meshes and walls for the benchmark's draw census (`RENDER-44`; nothing
    /// calls it on a frame path): `(MultiMesh, instance, instances as sent, whether the instance
    /// draws, whether it is a wall, the patch node, whether the set draws)`. `O(sets' meshes)`.
    pub(crate) fn census(
        &self,
    ) -> impl Iterator<Item = (Handle, Handle, u32, bool, bool, i64, bool)> + '_ {
        self.sets.iter().flat_map(|objects| {
            let walls = std::iter::once((&objects.walls, true));
            objects.parts.iter().map(|part| (part, false)).chain(walls).map(|(part, wall)| {
                (
                    part.multimesh,
                    part.instance,
                    part.count.unwrap_or(0),
                    part.drawn,
                    wall,
                    objects.parent,
                    objects.shown,
                )
            })
        })
    }

    /// Set `set`'s walls: instances drawn (as sent), and whether they draw.
    pub(crate) fn walls(&self, set: u32) -> (u32, bool) {
        let walls = &self.sets[set as usize].walls;
        (walls.count.unwrap_or(0), walls.drawn)
    }

    /// Whether set `set` draws (its root shown) under patch node `node`.
    pub(crate) fn drawn_under(&self, set: u32, node: i64) -> bool {
        let objects = &self.sets[set as usize];
        objects.shown && objects.parent == node
    }

    /// The patch node set `set` stands under (0: none) and whether it draws.
    pub(crate) fn parent(&self, set: u32) -> (i64, bool) {
        let objects = &self.sets[set as usize];
        (objects.parent, objects.shown)
    }
}

// A patch node's position: its rectangle's centre on the ground plane (`terrain.gd` places each
// patch node there).
fn patch_origin(patch: &Patch) -> [f32; 3] {
    let centre =
        |at: usize| (f64::from(patch.rect[at]) + f64::from(patch.rect[at + 2]) * 0.5) as f32;
    [centre(0), 0.0, centre(1)]
}

// The rectangle in half-metres, as `_begin_cells` rounds it (`Rect2.end` in `real_t`).
fn rect_hm(rect: [f32; 4]) -> [i32; 4] {
    let half = |value: f32| (f64::from(value) * 2.0).round() as i32;
    [
        half(rect[0]),
        half(rect[1]),
        half(rect[0] + rect[2]),
        half(rect[1] + rect[3]),
    ]
}

#[cfg(test)]
mod tests;
