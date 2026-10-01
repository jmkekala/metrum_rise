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

// Landscape plants are bush variants past the six the generator scatters; tree_species.gd
// lists them in LANDSCAPE_FORMS in this order. They are only ever planted by name.
const LILAC: &[u8] = &[6];
const SPIREA: &[u8] = &[7];
const ROSE: &[u8] = &[8];
const COTONEASTER: &[u8] = &[9];
const MUGO: &[u8] = &[10];
const JUNIPER: &[u8] = &[11];
const HEDGE_LOW: &[u8] = &[12];
const HEDGE_MID: &[u8] = &[13];
const HEDGE_TALL: &[u8] = &[14];
/// First bush variant that is a landscape plant rather than generated ground cover.
pub(crate) const LANDSCAPE_FIRST_VARIANT: u8 = 6;
/// First bush variant that is a clipped hedge module, planted along a line.
pub(crate) const HEDGE_FIRST_VARIANT: u8 = 12;
// A yard shrub is planted near its authored size; the generator's band would span a lilac
// from 2.6 m to 4.7 m tall before the renderer's own spread.
const SHRUB_SCALE: (f32, f32) = (0.85, 1.10);

/// One weighted choice inside a preset's mix.
struct MixEntry {
    // Relative to the sum over the preset's entries, so the weights need no common total.
    weight: u16,
    species: u8,
    // Meshes this entry may plant. An empty set leaves the renderer its seeded choice, which
    // is what every stroke did before a brush could name one tree.
    variants: &'static [u8],
}

/// What one brush stroke plants, and its candidate acceptance.
pub(super) struct BrushPreset {
    /// Fraction of otherwise clear dart proposals that keep a plant.
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
    BrushPreset { accept: 0.36, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 0, variants: &[] }] },
    BrushPreset { accept: 0.36, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 1, variants: &[] }] },
    BrushPreset { accept: 0.18, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 2, variants: &[] }] },
    BrushPreset { accept: 0.18, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 3, variants: &[] }] },
    // 4 Pine, 5 Spruce, 6 Birch, 7 Aspen: one named tree, pinned over the seed's choice.
    BrushPreset { accept: 0.36, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 0, variants: PINE_OR_BIRCH }] },
    BrushPreset { accept: 0.36, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 0, variants: SPRUCE_OR_ASPEN }] },
    BrushPreset { accept: 0.36, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 1, variants: PINE_OR_BIRCH }] },
    BrushPreset { accept: 0.36, scale: DEFAULT_SCALE, mix: &[MixEntry { weight: 1, species: 1, variants: SPRUCE_OR_ASPEN }] },
    // 8, 9 and 10 are the three mixes, which the brush numbers rather than names.
    //
    // 8 is a managed stand. The weights are Finnish growing stock: pine leads, spruce follows,
    // birch is most of the rest and aspen is an accent. Its calibrated acceptance targets
    // about 530 stems/ha, which is inside the real 400-700 of a managed stand.
    BrushPreset {
        accept: 0.25,
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
        accept: 0.0125,
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
        accept: 0.06,
        scale: (0.45, 0.75),
        mix: &[
            MixEntry { weight: 65, species: 1, variants: PINE_OR_BIRCH },
            MixEntry { weight: 35, species: 0, variants: PINE_OR_BIRCH },
        ],
    },
    // 11 to 16 are yard shrubs: one named plant each, at about one plant per 6 to 12 m².
    BrushPreset { accept: 0.03, scale: SHRUB_SCALE, mix: &[MixEntry { weight: 1, species: 2, variants: LILAC }] },
    BrushPreset { accept: 0.06, scale: SHRUB_SCALE, mix: &[MixEntry { weight: 1, species: 2, variants: SPIREA }] },
    BrushPreset { accept: 0.06, scale: SHRUB_SCALE, mix: &[MixEntry { weight: 1, species: 2, variants: ROSE }] },
    BrushPreset { accept: 0.05, scale: SHRUB_SCALE, mix: &[MixEntry { weight: 1, species: 2, variants: COTONEASTER }] },
    BrushPreset { accept: 0.04, scale: SHRUB_SCALE, mix: &[MixEntry { weight: 1, species: 2, variants: MUGO }] },
    BrushPreset { accept: 0.06, scale: SHRUB_SCALE, mix: &[MixEntry { weight: 1, species: 2, variants: JUNIPER }] },
    // 17, 18 and 19 are hedge modules, which plant_vegetation_line lays end to end at full size.
    BrushPreset { accept: 0.0, scale: (1.0, 1.0), mix: &[MixEntry { weight: 1, species: 2, variants: HEDGE_LOW }] },
    BrushPreset { accept: 0.0, scale: (1.0, 1.0), mix: &[MixEntry { weight: 1, species: 2, variants: HEDGE_MID }] },
    BrushPreset { accept: 0.0, scale: (1.0, 1.0), mix: &[MixEntry { weight: 1, species: 2, variants: HEDGE_TALL }] },
];

/// Borrows one preset, or `None` when the ordinal names none.
pub(super) fn preset(index: i64) -> Option<&'static BrushPreset> {
    usize::try_from(index).ok().and_then(|i| PRESETS.get(i))
}

impl BrushPreset {
    /// Ranks one proposal after clump and brush-edge thinning; the preset keeps it below 1.
    ///
    /// The rank is a pure function of the cell and the stream's salt. Proposals are planted in
    /// rising rank, so a stamp that stops at its limit plants a prefix of the full stand and the
    /// next stamp continues it, however the stamps are split.
    pub(super) fn rank(&self, x: i32, z: i32, salt: u32, influence: f32) -> f32 {
        unit(x, z, salt.wrapping_add(7)) / (self.accept * influence)
    }

    /// Occupancy and proposal budget shared by every species in this preset.
    pub(super) fn class(&self) -> PlantClass {
        let entry = &self.mix[0];
        PlantClass::of(entry.species, entry.variants.first().map_or(0, |v| v + 1))
    }

    /// Whether this preset plants hedge modules, which only a line may lay.
    pub(super) fn is_hedge(&self) -> bool {
        self.mix[0].species == 2
            && self.mix[0].variants.first().is_some_and(|&v| v >= HEDGE_FIRST_VARIANT)
    }

    /// Picks the species and the biased variant pin for one proposal.
    pub(super) fn plant(&self, x: i32, z: i32, salt: u32) -> (u8, u8) {
        // Eight of the eleven presets offer one choice, and a weighted pick over one entry can
        // only return that entry. Skipping it drops a hash from every proposal of every
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

    // One mix entry's species and the biased mesh pin it plants at one proposal.
    fn pick_variant(&self, entry: &MixEntry, x: i32, z: i32, salt: u32) -> (u8, u8) {
        if entry.variants.is_empty() {
            return (entry.species, VARIANT_FROM_SEED);
        }
        // Indexed rather than filtered: a filter would allocate once per proposal, and
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

/// Independent exclusion groups; generator layers remain save and rendering identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlantClass {
    /// Conifer and broadleaf stems; crowns may overlap.
    Tree,
    /// Bush catalogue, including low plants and saplings.
    Ground,
    /// Named yard shrubs and hedge modules: planted by hand beside roads and houses.
    Landscape,
    /// Decorative rocks.
    Rock,
}

impl PlantClass {
    /// Derives occupancy from a species and its biased variant pin, without changing stored
    /// species or owner cells.
    pub(super) fn of(species: u8, variant: u8) -> Self {
        match species {
            0 | 1 => Self::Tree,
            2 if variant > LANDSCAPE_FIRST_VARIANT => Self::Landscape,
            2 => Self::Ground,
            _ => Self::Rock,
        }
    }

    /// Minimum center distance in metres for new plants of this class.
    pub(super) fn spacing(self) -> f32 {
        match self {
            Self::Tree => 2.5,
            Self::Ground => 0.8,
            Self::Landscape => 0.5,
            Self::Rock => 3.0,
        }
    }

    /// Most plants one stamp of `radius` adds: one per 160 spacing squared, which is 10 stems/ha
    /// for trees, and at least one. Holding or repeating a stamp thickens a stand by that step
    /// until the preset's own density is reached.
    pub(super) fn stamp_limit(self, radius: f32) -> usize {
        let area = std::f32::consts::PI * radius * radius;
        (area / (160.0 * self.spacing() * self.spacing())).ceil().max(1.0) as usize
    }

    /// Radius limit bounds two proposals per cell to at most 169,362 darts.
    pub(super) fn max_radius(self) -> f32 {
        match self {
            Self::Ground | Self::Landscape => 64.0,
            _ => 256.0,
        }
    }

    /// Footprint appropriate to the species, independent of its saved owner layer.
    pub(super) fn clearance_layer(self) -> crate::simulation::vegetation::edits::VegetationLayer {
        use crate::simulation::vegetation::edits::VegetationLayer;
        match self {
            Self::Tree => VegetationLayer::Canopy,
            _ => VegetationLayer::Understory,
        }
    }
}
