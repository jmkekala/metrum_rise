// SPDX-License-Identifier: GPL-2.0-only

//! The road chunk hydration's source (`RENDER-36`, `RENDER-32-P`, `RENDER-52` part 2): a load's
//! whole network taken once, as the full road snapshot took it, and installed a part at a time
//! as the streaming core's parts reach it (`road_chunk_hydration.gd`), instead of every chunk in
//! one call.
//!
//! The chunks at the head of the stream's order are encoded in the `RenderingServer`'s own
//! layout on the background pool ([`RoadChunkSurfaces`], at most the stream's `PREPARE_AHEAD`
//! waiting or in flight); each part on the main thread hands one finished surface to
//! `RenderingServer.mesh_add_surface` on the chunk's hidden mesh, and the chunk's swap takes the
//! finished meshes with its lights and signs to the road tool's commit. The meshes are
//! `ArrayMesh` resources used as the server's mesh handles (their surfaces are added here, not
//! through the resource), so a dropped build or an installed chunk's node frees its mesh with the
//! resource.
//!
//! A chunk a later network revision changed is installed by that revision's own update: its
//! acknowledgement takes the revision's chunks out of the source, so the hydration never draws
//! an older version over a newer one.

use super::*;
use crate::nodes::sim::render::stream::layers::roads::surfaces::{
    LAYER_COUNT, RoadChunkSurfaces, RoadSurface,
};
use crate::nodes::sim::render::stream::layers::roads::RoadKey;
use crate::nodes::sim::render::stream::prepare::PrepareQueue;
use crate::simulation::network::render::{NetworkMeshData, RoadChunkMap};
use godot::classes::{ArrayMesh, Material, RenderingServer};
use std::sync::PoisonError;

/// Predicted cost of one byte handed to `RenderingServer.mesh_add_surface`, microseconds
/// (`RENDER-52` part 2's measurement; the copy into a `PackedByteArray` included).
pub(crate) const ROAD_US_PER_BYTE: f64 = 0.0005;
/// Predicted fixed cost of a surface part, microseconds.
pub(crate) const ROAD_SURFACE_US: f64 = 20.0;
/// Predicted cost of a chunk's street lights, microseconds a lamp (`street_lights.gd`'s batch
/// nodes; 7.5-9.8 us measured on the 30k city's load).
pub(crate) const ROAD_US_PER_LIGHT: f64 = 9.0;
/// The slice a street lights part runs for, microseconds (`road_chunk_hydration.gd`'s
/// `LIGHTS_SLICE_US`): a chunk's lamps make their nodes over as many parts as it takes.
pub(crate) const ROAD_LIGHTS_SLICE_US: f64 = 1000.0;
/// Predicted cost of a chunk's swap (its node and signs), microseconds.
pub(crate) const ROAD_SWAP_US: f64 = 150.0;
/// [`SimulationNode::run_road_mesh_hydration_part`]'s answers.
const PART_SWAP: i64 = 0;
const PART_SURFACE: i64 = 1;
const PART_LIGHTS: i64 = 2;
// Encode jobs out at once.
const MAX_IN_FLIGHT: usize = 4;
// The lining's shadow caster's material, after the chunk layers' (`set_road_hydration_materials`).
const SHADOW_MATERIAL: usize = LAYER_COUNT;
// `RenderingServer.PRIMITIVE_TRIANGLES`.
const PRIMITIVE_TRIANGLES: i64 = 3;

// Encoded chunks landed from the pool: key, request serial, surfaces.
type Landed = Arc<Mutex<Vec<(RoadKey, u64, RoadChunkSurfaces)>>>;

thread_local! {
    // The mesh ledger (`RENDER-52` part 2's teardown count; off unless a test or probe turns it
    // on) of the meshes the hydrations made on this (the main) thread.
    static MESH_LEDGER: std::cell::RefCell<Option<MeshLedger>> = const { std::cell::RefCell::new(None) };
}

// The meshes made since the ledger was turned on and the instance ids of those not seen freed.
#[derive(Default)]
struct MeshLedger {
    made: i64,
    alive: Vec<InstanceId>,
}

// A hydration mesh, recorded in the ledger while it is on. `O(1)`.
fn new_mesh() -> Gd<ArrayMesh> {
    let mesh = ArrayMesh::new_gd();
    MESH_LEDGER.with_borrow_mut(|ledger| {
        if let Some(ledger) = ledger {
            ledger.made += 1;
            ledger.alive.push(mesh.instance_id());
        }
    });
    mesh
}

// A chunk being built: its encoded surfaces, its hidden meshes, the surfaces added so far (the
// layers', then the lining caster's) and whether its lights were handed out.
struct Build {
    surfaces: RoadChunkSurfaces,
    mesh: Gd<ArrayMesh>,
    shadow: Option<Gd<ArrayMesh>>,
    added: usize,
    lights: bool,
}

impl Build {
    fn parts(&self) -> usize {
        self.surfaces.layers.len() + usize::from(self.surfaces.lining_shadow.is_some())
    }

    fn finished(&self) -> bool {
        self.added == self.parts() && self.lights
    }

    fn next_surface(&self) -> Option<&RoadSurface> {
        self.surfaces
            .layers
            .get(self.added)
            .map(|(_, surface)| surface)
            .or_else(|| {
                (self.added == self.surfaces.layers.len())
                    .then_some(self.surfaces.lining_shadow.as_ref())
                    .flatten()
            })
    }
}

/// The network a hydration installs from: the published chunks at one road mesh generation, and
/// the chunks being encoded and built.
pub(crate) struct RoadHydrationSource {
    generation: u64,
    chunks: RoadChunkMap,
    span: f32,
    origin: [f32; 2],
    jobs: PrepareQueue<RoadKey>,
    requested: HashMap<RoadKey, u64>,
    landed: Landed,
    builds: HashMap<RoadKey, Build>,
    // The chunk layers' materials, then the lining caster's.
    materials: Vec<Gd<Material>>,
}

impl RoadHydrationSource {
    fn chunk(&self, key: RoadKey) -> Option<&Arc<NetworkMeshData>> {
        self.chunks.get(&(key[0], key[1]))
    }

    // Takes the landed encodings, drops the builds and requests outside `head`, and starts
    // encodings for `head`'s chunks that have neither, nearest first. `O(landed + head)`.
    fn prepare(&mut self, head: &[RoadKey]) {
        let landed = std::mem::take(&mut *self.landed.lock().unwrap_or_else(PoisonError::into_inner));
        for (key, serial, surfaces) in landed {
            if self.jobs.finish(key, serial) && self.requested.remove(&key).is_some() {
                self.builds.insert(
                    key,
                    Build {
                        surfaces,
                        mesh: new_mesh(),
                        shadow: None,
                        added: 0,
                        lights: false,
                    },
                );
            }
        }
        self.builds.retain(|key, _| head.contains(key));
        let jobs = &mut self.jobs;
        self.requested.retain(|key, _| {
            let keep = head.contains(key);
            if !keep {
                jobs.cancel(*key);
            }
            keep
        });
        for &key in head {
            if self.builds.contains_key(&key) || self.requested.contains_key(&key) {
                continue;
            }
            let Some(mesh) = self.chunk(key).cloned() else {
                continue;
            };
            let Some(serial) = self.jobs.start(key) else {
                break;
            };
            self.requested.insert(key, serial);
            let landed = Arc::clone(&self.landed);
            crate::thread_pools::spawn(move || {
                let surfaces = RoadChunkSurfaces::encode(&mesh);
                landed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((key, serial, surfaces));
            });
        }
    }

    // Adds `key`'s next surface to its hidden mesh: false when none is left (or no build).
    fn add_surface(&mut self, key: RoadKey) -> bool {
        let Some(build) = self.builds.get_mut(&key) else {
            return false;
        };
        let Some(surface) = build.next_surface() else {
            return false;
        };
        let layer = build.surfaces.layers.get(build.added).map(|&(layer, _)| layer);
        let material = self.materials.get(layer.unwrap_or(SHADOW_MATERIAL)).map(|m| m.get_rid());
        let mut dict = VarDictionary::new();
        dict.set("primitive", PRIMITIVE_TRIANGLES);
        dict.set("format", surface.format as i64);
        dict.set("vertex_data", PackedByteArray::from(surface.vertex_data.as_slice()));
        if !surface.attribute_data.is_empty() {
            dict.set("attribute_data", PackedByteArray::from(surface.attribute_data.as_slice()));
        }
        dict.set("vertex_count", i64::from(surface.vertex_count));
        dict.set("aabb", aabb(surface));
        dict.set("uv_scale", Vector4::ZERO);
        let mesh = match layer {
            Some(_) => build.mesh.get_rid(),
            None => build.shadow.get_or_insert_with(new_mesh).get_rid(),
        };
        let mut server = RenderingServer::singleton();
        let index = server.mesh_get_surface_count(mesh);
        server.mesh_add_surface(mesh, &dict);
        if let Some(material) = material {
            server.mesh_surface_set_material(mesh, index, material);
        }
        build.added += 1;
        true
    }
}

fn aabb(surface: &RoadSurface) -> Aabb {
    let [x, y, z, w, h, d] = surface.aabb;
    Aabb::new(Vector3::new(x, y, z), Vector3::new(w, h, d))
}

#[godot_api(secondary)]
impl SimulationNode {
    /// Takes the whole road network for a hydration when its road mesh generation is
    /// `expected_generation` (else returns an empty dictionary and takes nothing): the
    /// `surface_generation`, `chunk_span_m`, `chunk_origin_x_m`, `chunk_origin_z_m` of the full
    /// road snapshot and `keys` (x, z pairs). Waits for the core once, as the full snapshot did;
    /// the chunk map is shared, `O(chunks)` to list. Any hydration taken before is dropped with
    /// its builds.
    #[func]
    pub fn begin_road_mesh_hydration(&mut self, expected_generation: i64) -> VarDictionary {
        let source = {
            let core = self.lock_core();
            if i64::try_from(core.cached_road_mesh_generation).ok() != Some(expected_generation) {
                return VarDictionary::new();
            }
            let (x, z) = core.transit_network.roadbed.chunk_origin_m();
            RoadHydrationSource {
                generation: core.cached_road_mesh_generation,
                chunks: core.cached_road_mesh_chunks.clone(),
                span: core.transit_network.roadbed.chunk_span_m(),
                origin: [x, z],
                jobs: PrepareQueue::new(MAX_IN_FLIGHT),
                requested: HashMap::new(),
                landed: Landed::default(),
                builds: HashMap::new(),
                materials: Vec::new(),
            }
        };
        let mut keys = PackedInt32Array::new();
        for &(x, z) in source.chunks.keys() {
            keys.push(x);
            keys.push(z);
        }
        let mut out = VarDictionary::new();
        out.set("surface_generation", expected_generation);
        out.set("chunk_span_m", source.span);
        out.set("chunk_origin_x_m", source.origin[0]);
        out.set("chunk_origin_z_m", source.origin[1]);
        out.set("keys", keys);
        self.road_hydration = Some(source);
        out
    }

    /// The materials the hydration's surfaces draw with: one per chunk layer in payload order,
    /// then the lining's shadow caster's. False, keeping none, when the count is not that or no
    /// hydration was taken.
    #[func]
    pub fn set_road_hydration_materials(&mut self, materials: Array<Gd<Material>>) -> bool {
        let Some(source) = self.road_hydration.as_mut() else {
            return false;
        };
        if materials.len() != LAYER_COUNT + 1 {
            return false;
        }
        source.materials = materials.iter_shared().collect();
        true
    }

    /// Asks for the surfaces of the chunks `keys` (x, z pairs, the head of the stream's order,
    /// nearest first) to be encoded on the background pool; builds of chunks not listed are
    /// dropped. `O(keys)`.
    #[func]
    pub fn prepare_road_mesh_hydration(&mut self, keys: PackedInt32Array) {
        if let Some(source) = self.road_hydration.as_mut() {
            source.prepare(keys.as_slice().as_chunks::<2>().0);
        }
    }

    /// Chunk `(x, z)`'s next part's predicted cost in microseconds: a surface by its bytes, its
    /// street lights by their count, or the swap; -1 while its surfaces are not encoded, -2 when
    /// the hydration has no such chunk (none began, it ended, or a newer revision took the
    /// chunk). `O(1)`.
    #[func]
    pub fn get_road_mesh_hydration_part_us(&self, x: i32, z: i32) -> f64 {
        let Some(source) = self.road_hydration.as_ref() else {
            return -2.0;
        };
        let Some(chunk) = source.chunk([x, z]) else {
            return -2.0;
        };
        match source.builds.get(&[x, z]) {
            None => -1.0,
            Some(build) => match build.next_surface() {
                Some(surface) => ROAD_SURFACE_US + surface.bytes() as f64 * ROAD_US_PER_BYTE,
                None if !build.lights => {
                    (chunk.street_lights.len() as f64 * ROAD_US_PER_LIGHT).min(ROAD_LIGHTS_SLICE_US)
                }
                None => ROAD_SWAP_US,
            },
        }
    }

    /// Runs chunk `(x, z)`'s next part that is Rust's: adds its next encoded surface to its
    /// hidden mesh (answers 1); else says its street lights are due (2: take them with
    /// `take_road_mesh_hydration_lights`); else the swap is next (0, also without a build).
    /// `O(the surface's bytes)`.
    #[func]
    pub fn run_road_mesh_hydration_part(&mut self, x: i32, z: i32) -> i64 {
        let Some(source) = self.road_hydration.as_mut() else {
            return PART_SWAP;
        };
        if source.add_surface([x, z]) {
            return PART_SURFACE;
        }
        match source.builds.get(&[x, z]) {
            Some(build) if !build.lights => PART_LIGHTS,
            _ => PART_SWAP,
        }
    }

    /// Chunk `(x, z)`'s street lights (five floats each, chunk-local base x, y, z, yaw, kind),
    /// installed by the chunk renderer as their own part ahead of the swap; the part is then done.
    /// Empty when the hydration has no such build. `O(lights)`.
    #[func]
    pub fn take_road_mesh_hydration_lights(&mut self, x: i32, z: i32) -> PackedFloat32Array {
        let Some(source) = self.road_hydration.as_mut() else {
            return PackedFloat32Array::new();
        };
        let Some(chunk) = source.chunk([x, z]).cloned() else {
            return PackedFloat32Array::new();
        };
        let Some(build) = source.builds.get_mut(&[x, z]) else {
            return PackedFloat32Array::new();
        };
        build.lights = true;
        SimCore::road_mesh_lights(&chunk)
    }

    /// Takes chunk `(x, z)`'s finished build for its swap: `mesh` (the layers' surfaces),
    /// `lining_mesh` (the lining's shadow caster, or null), per layer in payload order
    /// `layer_surfaces` (its surface, -1 when empty), `layer_counts` and `layer_formats`, and the
    /// payload's `chunk_x`, `chunk_z`, `removed` (false) and `road_signs` (its street lights
    /// were their own part). An empty dictionary when the hydration has no such chunk or its
    /// build is not finished.
    #[func]
    pub fn take_road_mesh_hydration_chunk(&mut self, x: i32, z: i32) -> VarDictionary {
        let Some(source) = self.road_hydration.as_mut() else {
            return VarDictionary::new();
        };
        let Some(chunk) = source.chunk([x, z]).cloned() else {
            return VarDictionary::new();
        };
        if source
            .builds
            .get(&[x, z])
            .is_none_or(|build| !build.finished())
        {
            return VarDictionary::new();
        }
        let Some(build) = source.builds.remove(&[x, z]) else {
            return VarDictionary::new();
        };
        let mut surfaces = PackedInt32Array::from(&[-1; LAYER_COUNT][..]);
        let mut counts = PackedInt32Array::from(&[0; LAYER_COUNT][..]);
        let mut formats = PackedInt64Array::from(&[0; LAYER_COUNT][..]);
        for (index, (layer, surface)) in build.surfaces.layers.iter().enumerate() {
            surfaces.as_mut_slice()[*layer] = index as i32;
            counts.as_mut_slice()[*layer] = surface.vertex_count as i32;
            formats.as_mut_slice()[*layer] = surface.format as i64;
        }
        let mut dict = VarDictionary::new();
        dict.set("mesh", build.mesh.to_variant());
        dict.set(
            "lining_mesh",
            build.shadow.as_ref().map_or_else(Variant::nil, ToGodot::to_variant),
        );
        dict.set("layer_surfaces", surfaces);
        dict.set("layer_counts", counts);
        dict.set("layer_formats", formats);
        dict.set("chunk_x", x);
        dict.set("chunk_z", z);
        dict.set("removed", false);
        dict.set("road_signs", SimCore::road_mesh_signs(&chunk));
        dict
    }

    /// Ends the hydration: its network and unfinished builds are let go.
    #[func]
    pub fn end_road_mesh_hydration(&mut self) {
        self.road_hydration = None;
    }

    /// Turns the mesh ledger on (`on`, empty) or off: while on, every mesh a hydration makes on
    /// this thread is recorded (the teardown count, `RENDER-52` part 2; tests and the teardown
    /// probe). `O(1)`.
    #[func]
    pub fn set_road_hydration_mesh_ledger(&mut self, on: bool) {
        MESH_LEDGER.set(on.then(MeshLedger::default));
    }

    /// The ledger's meshes still alive (instance ids, in the order made; the freed ones leave
    /// the ledger): those an installed chunk draws, those of builds not swapped in yet, and any
    /// leaked. A freed mesh frees its `RenderingServer` mesh with it, and the hydration makes no
    /// other server handle. Empty while the ledger is off. `O(meshes recorded)`.
    #[func]
    pub fn get_road_hydration_meshes_alive(&self) -> PackedInt64Array {
        MESH_LEDGER.with_borrow_mut(|ledger| {
            let Some(ledger) = ledger else {
                return PackedInt64Array::new();
            };
            ledger.alive.retain(|id| id.lookup_validity());
            ledger.alive.iter().map(|id| id.to_i64()).collect()
        })
    }

    /// The meshes the hydrations made since the ledger was turned on (alive or freed); 0 while
    /// it is off. `O(1)`.
    #[func]
    pub fn get_road_hydration_meshes_made(&self) -> i64 {
        MESH_LEDGER.with_borrow(|ledger| ledger.as_ref().map_or(0, |ledger| ledger.made))
    }

    /// The meshes the hydration's builds hold (not swapped in yet). `O(builds)`.
    #[func]
    pub fn get_road_hydration_build_meshes(&self) -> i64 {
        self.road_hydration.as_ref().map_or(0, |source| {
            source
                .builds
                .values()
                .map(|build| 1 + i64::from(build.shadow.is_some()))
                .sum()
        })
    }

    /// The published road chunk `(x, z)`'s surfaces as the hydration encodes them, for tests
    /// against `RenderingServer.mesh_get_surface`: per non-empty layer `layer`, `format`,
    /// `vertex_count`, `vertex_data`, `attribute_data` and `aabb`, then the lining's shadow
    /// caster with `layer` -1. Empty for no such chunk. Waits for the core once.
    #[func]
    pub fn get_road_chunk_surfaces(&self, x: i32, z: i32) -> VarArray {
        let chunk = self.lock_core().cached_road_mesh_chunks.get(&(x, z)).cloned();
        let mut out = VarArray::new();
        let Some(chunk) = chunk else {
            return out;
        };
        let surfaces = RoadChunkSurfaces::encode(&chunk);
        let entries = surfaces
            .layers
            .iter()
            .map(|(layer, surface)| (*layer as i64, surface))
            .chain(surfaces.lining_shadow.iter().map(|surface| (-1, surface)));
        for (layer, surface) in entries {
            let mut dict = VarDictionary::new();
            dict.set("layer", layer);
            dict.set("format", surface.format as i64);
            dict.set("vertex_count", i64::from(surface.vertex_count));
            dict.set("vertex_data", PackedByteArray::from(surface.vertex_data.as_slice()));
            dict.set("attribute_data", PackedByteArray::from(surface.attribute_data.as_slice()));
            dict.set("aabb", aabb(surface));
            out.push(&dict.to_variant());
        }
        out
    }
}

impl SimulationNode {
    // An acknowledged network revision newer than the hydration's installed `chunks` itself:
    // they leave the hydration. `O(chunks log n)`.
    pub(super) fn road_hydration_acknowledged(
        &mut self,
        generation: u64,
        chunks: &std::collections::BTreeSet<crate::simulation::network::roadbed::SurfaceChunkKey>,
    ) {
        if let Some(source) = self.road_hydration.as_mut()
            && generation > source.generation
        {
            for key in chunks {
                source.chunks.remove(key);
                source.builds.remove(&[key.0, key.1]);
            }
        }
    }
}
