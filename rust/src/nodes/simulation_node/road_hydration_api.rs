// SPDX-License-Identifier: GPL-2.0-only

//! The road chunk hydration's source (`RENDER-36`, `RENDER-32-P`): a load's whole network taken
//! once, as the full road snapshot took it, and packed one chunk at a time as the streaming
//! core's units reach it (`road_chunk_hydration.gd`), instead of every chunk in one call.
//!
//! A chunk a later network revision changed is installed by that revision's own update: its
//! acknowledgement takes the revision's chunks out of the source, so the hydration never draws
//! an older version over a newer one.

use super::*;
use crate::simulation::network::render::RoadChunkMap;

/// The network a hydration installs from: the published chunks at one road mesh generation.
pub(crate) struct RoadHydrationSource {
    generation: u64,
    chunks: RoadChunkMap,
    span: f32,
    origin: [f32; 2],
}

#[godot_api(secondary)]
impl SimulationNode {
    /// Takes the whole road network for a hydration when its road mesh generation is
    /// `expected_generation` (else returns an empty dictionary and takes nothing): the
    /// `surface_generation`, `chunk_span_m`, `chunk_origin_x_m`, `chunk_origin_z_m` of the full
    /// road snapshot, `keys` (x, z pairs) and `vertices` per chunk. Waits for the core once, as
    /// the full snapshot did; the chunk map is shared, `O(chunks)` to list.
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
            }
        };
        let mut keys = PackedInt32Array::new();
        let mut vertices = PackedInt64Array::new();
        for (&(x, z), mesh) in source.chunks.iter() {
            keys.push(x);
            keys.push(z);
            vertices.push(mesh.vertex_count() as i64);
        }
        let mut out = VarDictionary::new();
        out.set("surface_generation", expected_generation);
        out.set("chunk_span_m", source.span);
        out.set("chunk_origin_x_m", source.origin[0]);
        out.set("chunk_origin_z_m", source.origin[1]);
        out.set("keys", keys);
        out.set("vertices", vertices);
        self.road_hydration = Some(source);
        out
    }

    /// One chunk of the hydration in the full road snapshot's chunk format, or an empty
    /// dictionary when the hydration has no such chunk (none began, it ended, or a newer
    /// revision took the chunk). `O(the chunk's vertices)`, no lock.
    #[func]
    pub fn get_road_mesh_hydration_chunk(&self, x: i32, z: i32) -> VarDictionary {
        self.road_hydration
            .as_ref()
            .and_then(|source| source.chunks.get(&(x, z)))
            .map_or_else(VarDictionary::new, |mesh| {
                let mut dict = SimCore::network_mesh_data_dict(mesh);
                dict.set("chunk_x", x);
                dict.set("chunk_z", z);
                dict.set("removed", false);
                dict
            })
    }

    /// Ends the hydration: its network is let go.
    #[func]
    pub fn end_road_mesh_hydration(&mut self) {
        self.road_hydration = None;
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
            }
        }
    }
}
