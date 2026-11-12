// SPDX-License-Identifier: GPL-2.0-only

//! The graded pages' slot uploads (`RENDER-32-K` part 2, `docs/terrain.md` "Pages and the row
//! atlas"), moved from `terrain_graded_pages.gd`'s `write_copy`: a placed tile's bake is written
//! into each copy the book gave it, its slot layer with the header's first channel set to its
//! run's first atlas layer (`residency.rs` `pack_slot`) and its row atlas run one 256-texel layer
//! at a time, through the stream backend ([`RenderBackend::texture_write_layer`]).
//!
//! The book stages the bake's bytes as the renderer places it; each `Write` effect of that take
//! keeps them (shared, not copied) and the renderer applies the writes as a run in their place
//! among the effects (effect 48), after the page arrays that effects before them made. The bytes
//! a page holds are a function of the writes in order, which the traces pin (`write` lines).
//! `O(slot bytes + rows)` per write, one backend call per slot layer and per atlas layer: the
//! `RenderingDevice` updates a texture by whole layers only. No allocation once the writes'
//! list has grown to a take's writes.

use super::pages::Copy;
use crate::nodes::sim::render::stream::backend::{Handle, RenderBackend};

/// Texels of a row atlas layer (`residency.rs` `ATLAS_WIDTH`).
pub(crate) const ATLAS_WIDTH: u32 = 256;
/// Bytes of a row atlas layer.
pub(crate) const ATLAS_LAYER_BYTES: usize = ATLAS_WIDTH as usize * 16;

/// Writes a bake (`texels`, `rows`) into `copy` of a page whose textures are `textures` (slot layers, row atlas)
/// and whose slot layers are `side` texels square plus the header line. `O(slot bytes + rows)`.
pub(crate) fn write_copy(
    backend: &mut impl RenderBackend,
    textures: [Handle; 2],
    side: u32,
    copy: Copy,
    texels: &[u8],
    rows: &[u8],
) {
    // The header's first texel (column 0 of row `side`), red channel: the run's first layer.
    let header = side as usize * side as usize * 16;
    let first = (copy.first as f32).to_le_bytes();
    backend.texture_write_layer(textures[0], copy.slot, [side, side + 1], &mut |bytes| {
        debug_assert_eq!(
            bytes.len(),
            texels.len(),
            "a bake's slot layer is its level's"
        );
        let length = bytes.len().min(texels.len());
        bytes[..length].copy_from_slice(&texels[..length]);
        bytes[header..header + 4].copy_from_slice(&first);
    });
    for (index, layer) in rows.as_chunks::<ATLAS_LAYER_BYTES>().0.iter().enumerate() {
        backend.texture_write_layer(
            textures[1],
            copy.first + index as u32,
            [ATLAS_WIDTH, 1],
            &mut |bytes| {
                bytes.copy_from_slice(layer);
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::sim::render::stream::backend::recorder::{Command, Recorder};

    #[test]
    fn a_copy_writes_its_slot_with_the_run_header_and_one_call_per_atlas_layer() {
        let mut backend = Recorder::new();
        let side = 3u32;
        let texels: Vec<u8> = (0..side * (side + 1) * 16).map(|byte| byte as u8).collect();
        let rows: Vec<u8> = (0..2 * ATLAS_LAYER_BYTES)
            .map(|byte| (byte / 7) as u8)
            .collect();
        let copy = Copy {
            slot: 5,
            first: 9,
            layers: 3,
        };
        write_copy(&mut backend, [100, 200], side, copy, &texels, &rows);
        let layers: Vec<(Handle, u32, [u32; 2])> = backend
            .commands
            .iter()
            .map(|command| match command {
                Command::TextureLayer(texture, layer, size, _) => (*texture, *layer, *size),
                other => panic!("only layer writes: {other:?}"),
            })
            .collect();
        assert_eq!(
            layers,
            [(100, 5, [3, 4]), (200, 9, [256, 1]), (200, 10, [256, 1])]
        );
        let slot = &backend.textures[&100][&5];
        let header = (side * side * 16) as usize;
        assert_eq!(&slot[..header], &texels[..header], "the lattice as baked");
        assert_eq!(
            &slot[header..header + 4],
            &9.0f32.to_le_bytes(),
            "the header names the run's first layer"
        );
        assert_eq!(
            &slot[header + 4..],
            &texels[header + 4..],
            "the header's other channels as baked"
        );
        assert_eq!(
            &backend.textures[&200][&10][..],
            &rows[ATLAS_LAYER_BYTES..],
            "the run's second layer"
        );
    }
}
