// SPDX-License-Identifier: GPL-2.0-only

//! Named brush presets: what one stroke plants, how densely, and at what size.

use crate::simulation::vegetation::edits::{VARIANT_COUNTS, VARIANT_FROM_SEED};
use crate::simulation::vegetation::unit;

/// Instance scale band the generator places every plant in.
///
/// A preset that keeps this band returns each plant's scale untouched, so a stroke that does
/// not dwarf its trees produces the same floats it produced before presets existed.
pub(super) const DEFAULT_SCALE: (f32, f32) = (0.75, 1.35);

// Two of every three conifer variants are pine and two of every three broadleaf variants are
// birch; the remaining third are spruce and aspen. The rule lives in tree_species.gd, which
// owns the meshes, and these sets mirror it for the twelve variants each of those species
// models. The bridge test vegetation_edit_test.gd paints each named tree and checks the
// variants that come back still satisfy that rule, so the two cannot drift apart silently.
const PINE_OR_BIRCH: &[u8] = &[0, 1, 3, 4, 6, 7, 9, 10];
const SPRUCE_OR_ASPEN: &[u8] = &[2, 5, 8, 11];

/// One weighted choice inside a preset's mix.
struct MixEntry {
    // Relative to the sum over the preset's entries, so the weights need no common total.
    weight: u16,
    species: u8,
    // Meshes this entry may plant. An empty set leaves the renderer its seeded choice, which
    // is what every stroke did before a brush could name one tree.
    variants: &'static [u8],
}

/// What one brush stroke plants, and how much of its lattice it keeps.
pub(super) struct BrushPreset {
    /// Fraction of otherwise clear lattice points that keep a plant.
    accept: f32,
    /// Instance scale band this preset places its plants in.
    scale: (f32, f32),
    mix: &'static [MixEntry],
}

// Ordinals are the brush's wire contract with GDScript and nothing else: no save stores one,
// so the dropdown is free to list them in whatever order reads best. The first four are the
// species the brush offered before presets existed and keep their old numbers.
pub(super) const PRESETS: &[BrushPreset] = &[
    // 0 Conifer, 1 Broadleaf, 2 Bush, 3 Rock: one species, and the mesh inside it still left
    // to the appearance seed. The brush does not offer the first two, because a conifer whose
    // mesh the seed picks is only an unnamed two-to-one mix of pine and spruce and the mixes
    // below write their ratios down. They stay here because they are what the generator plants
    // and therefore what a repaint of a cleared cell has to match to collapse back to no edit.
    BrushPreset { accept: 1.0, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 0, variants: &[] }] },
    BrushPreset { accept: 1.0, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 1, variants: &[] }] },
    BrushPreset { accept: 1.0, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 2, variants: &[] }] },
    BrushPreset { accept: 1.0, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 3, variants: &[] }] },
    // 4 Pine, 5 Spruce, 6 Birch, 7 Aspen: one named tree, pinned over the seed's choice.
    BrushPreset { accept: 1.0, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 0, variants: PINE_OR_BIRCH }] },
    BrushPreset { accept: 1.0, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 0, variants: SPRUCE_OR_ASPEN }] },
    BrushPreset { accept: 1.0, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 1, variants: PINE_OR_BIRCH }] },
    BrushPreset { accept: 1.0, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 1, variants: SPRUCE_OR_ASPEN }] },
    // 8, 9 and 10 are the three mixes, which the brush numbers rather than names.
    //
    // 8 is a managed stand. The weights are Finnish growing stock: pine leads, spruce follows,
    // birch is most of the rest and aspen is an accent. At 0.85 of a 4 m lattice this is
    // about 530 stems/ha, which is inside the real 400-700 of a managed stand.
    BrushPreset {
        accept: 0.85,
        scale: DEFAULT_SCALE,
        mix: &[
            MixEntry { weight: 50, species: 0, variants: PINE_OR_BIRCH },
            MixEntry { weight: 30, species: 0, variants: SPRUCE_OR_ASPEN },
            MixEntry { weight: 15, species: 1, variants: PINE_OR_BIRCH },
            MixEntry { weight: 5, species: 1, variants: SPRUCE_OR_ASPEN },
        ],
    },
    // 9 is scattered broadleaf-led trees, about 62 stems/ha, which reads as standing apart
    // rather than as a stand, at a larger scale band because a tree with room around it is
    // larger than one that grew up under a canopy.
    BrushPreset {
        accept: 0.10,
        scale: (0.95, 1.45),
        mix: &[
            MixEntry { weight: 45, species: 1, variants: PINE_OR_BIRCH },
            MixEntry { weight: 25, species: 1, variants: SPRUCE_OR_ASPEN },
            MixEntry { weight: 20, species: 0, variants: PINE_OR_BIRCH },
            MixEntry { weight: 10, species: 0, variants: SPRUCE_OR_ASPEN },
        ],
    },
    // 10 is small trees at moderate spacing: a narrow birch-and-pine set at about 220
    // stems/ha and a lowered scale band. It is the density dial and the size dial moving
    // together, and it does not yet read as any particular real place.
    BrushPreset {
        accept: 0.35,
        scale: (0.45, 0.75),
        mix: &[
            MixEntry { weight: 65, species: 1, variants: PINE_OR_BIRCH },
            MixEntry { weight: 35, species: 0, variants: PINE_OR_BIRCH },
        ],
    },
];

/// Borrows one preset, or `None` when the ordinal names none.
pub(super) fn preset(index: i64) -> Option<&'static BrushPreset> {
    usize::try_from(index).ok().and_then(|i| PRESETS.get(i))
}

impl BrushPreset {
    /// Whether this preset keeps the lattice point at one cell of a stream's own grid.
    ///
    /// The decision is a pure function of the cell and the stream's salt, so a stroke thins
    /// to the same plants however its points are ordered and however often it is repeated.
    pub(super) fn keeps(&self, x: i32, z: i32, salt: u32) -> bool {
        self.accept >= 1.0 || unit(x, z, salt.wrapping_add(7)) < self.accept
    }

    /// Picks the species and the biased variant pin for one lattice point.
    pub(super) fn plant(&self, x: i32, z: i32, salt: u32) -> (u8, u8) {
        // Eight of the eleven presets offer one choice, and a weighted pick over one entry can
        // only return that entry. Skipping it drops a hash from every lattice point of every
        // named-tree stroke; the pick it replaces would land in the same place.
        if let [only] = self.mix {
            return self.pick_variant(only, x, z, salt);
        }
        let total: u32 = self.mix.iter().map(|entry| u32::from(entry.weight)).sum();
        let mut pick = (unit(x, z, salt.wrapping_add(8)) * total as f32) as u32;
        // The mix is a handful of entries, so a scan is cheaper than any index over it. The
        // last entry also absorbs a pick that a rounded multiply pushed to the total.
        let entry = self
            .mix
            .iter()
            .find(|entry| {
                let weight = u32::from(entry.weight);
                if pick < weight {
                    return true;
                }
                pick -= weight;
                false
            })
            .unwrap_or(&self.mix[self.mix.len() - 1]);
        self.pick_variant(entry, x, z, salt)
    }

    // One mix entry's species and the biased mesh pin it plants at one lattice point.
    fn pick_variant(&self, entry: &MixEntry, x: i32, z: i32, salt: u32) -> (u8, u8) {
        if entry.variants.is_empty() {
            return (entry.species, VARIANT_FROM_SEED);
        }
        // Indexed rather than filtered: a filter would allocate once per lattice point, and
        // this runs for every point of every stamp. The clamp absorbs a rounded multiply that
        // reached the length.
        let last = entry.variants.len() - 1;
        let index = (unit(x, z, salt.wrapping_add(9)) * entry.variants.len() as f32) as usize;
        let variant = entry.variants[index.min(last)];
        // A species that models fewer variants than the set names cannot pin one of them, so
        // it falls back to the seed rather than to a mesh that does not exist.
        if variant >= VARIANT_COUNTS[entry.species as usize] {
            return (entry.species, VARIANT_FROM_SEED);
        }
        (entry.species, variant + 1)
    }

    /// Maps a generator scale into this preset's band, returning it unchanged at the default.
    pub(super) fn size(&self, scale: f32) -> f32 {
        if self.scale == DEFAULT_SCALE {
            return scale;
        }
        let t = (scale - DEFAULT_SCALE.0) / (DEFAULT_SCALE.1 - DEFAULT_SCALE.0);
        self.scale.0 + t * (self.scale.1 - self.scale.0)
    }
}
