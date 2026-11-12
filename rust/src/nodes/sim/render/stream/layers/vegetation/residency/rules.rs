// SPDX-License-Identifier: GPL-2.0-only

//! The vegetation's distance rules in `vegetation.gd`'s arithmetic (`RENDER-32-F`): patch
//! centres, origins and distances in Godot's single precision `Vector2` operations (the engine's
//! operation order, no fused multiply-add, as `godot_math`), every comparison in GDScript's double
//! precision `float`. So a tier, a band and a camera cell here are the ones the script chose, bit
//! for bit (the golden replay of the stream benchmark's paths holds it).

use super::super::PatchKey;
use crate::nodes::sim::render::stream::rings::Cell;
use crate::nodes::sim::render::vegetation_product::ProductSettings;

/// Half the diagonal of a square patch, per metre of side (`PATCH_HALF_DIAGONAL`). A patch is one
/// instance, so every conservative range test measures from the patch centre out to its farthest
/// corner.
pub(crate) const HALF_DIAGONAL: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Richer content is kept this far past its entry distance (`BAND_HYSTERESIS_M`).
pub(crate) const BAND_HYSTERESIS_M: f64 = 64.0;
/// What a margin to a band boundary gives up for single precision distances over kilometres.
pub(crate) const MARGIN_SLACK_M: f64 = 0.01;

/// The residency's settings, from `vegetation.gd` (its overrides included).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResidencySettings {
    /// The draw distances and density the products are built with.
    pub(crate) product: ProductSettings,
    /// Vegetation patches per terrain patch along one axis (`patch_subdivision()`), 1 to 8.
    pub(crate) subdivision: i32,
    /// Requests a frame (`upload_budget_override`, else the subdivision squared).
    pub(crate) upload_budget: usize,
    /// The sun's shadow range (`SceneLightingConfig.shadow_max_distance_m()`).
    pub(crate) shadow_max_m: f64,
    /// The world's size, metres (`get_terrain_world_size()`).
    pub(crate) world: [f32; 2],
}

impl ResidencySettings {
    /// The bridge's packing: the product's six lanes (density, canopy near, crossfade begin,
    /// canopy far, bush range, rock range), then subdivision, upload budget (0: the subdivision
    /// squared), shadow range, world width and depth. `None` when malformed.
    pub(crate) fn from_lanes(lanes: &[f64]) -> Option<Self> {
        let product = ProductSettings::from_lanes(lanes)?;
        let [subdivision, budget, shadow_max_m, world_x, world_z] =
            *lanes.get(6..)?.first_chunk::<5>()?;
        let subdivision = subdivision as i32;
        if !(1..=8).contains(&subdivision) || !(world_x > 0.0 && world_z > 0.0) {
            return None;
        }
        let budget = budget as usize;
        Some(Self {
            product,
            subdivision,
            upload_budget: if budget >= 1 {
                budget
            } else {
                (subdivision * subdivision) as usize
            },
            shadow_max_m,
            world: [world_x as f32, world_z as f32],
        })
    }

    /// Distance within which a terrain block is carried on the fine grid (`_fine_tier_radius`).
    pub(crate) fn fine_radius(&self) -> f64 {
        let p = &self.product;
        p.near_m.max(p.bush_range_m.max(p.rock_range_m))
    }
}

/// The three bands of a patch: understory, near canopy, shadow caster.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Bands {
    /// The understory grid is scattered.
    pub(crate) understory: bool,
    /// The near canopy meshes are built.
    pub(crate) near_band: bool,
    /// The patch casts (`ShadowCaster.IMPOSTOR`).
    pub(crate) caster: bool,
}

/// The side of the patch `key` names, metres (`_key_span`).
pub(crate) fn key_span(key: PatchKey, terrain_span: f64) -> f64 {
    terrain_span / f64::from(key[2].max(1))
}

/// The centre of the patch `key` (`Vector2(key.x, key.y) * span - world * 0.5 + Vector2.ONE *
/// span * 0.5`).
pub(crate) fn key_centre(key: PatchKey, terrain_span: f64, world: [f32; 2]) -> [f32; 2] {
    let span = key_span(key, terrain_span) as f32;
    let half = span * 0.5;
    [
        key[0] as f32 * span - world[0] * 0.5 + half,
        key[1] as f32 * span - world[1] * 0.5 + half,
    ]
}

/// The minimum corner of the patch `key` (`Vector2(key.x, key.y) * span - world * 0.5`).
pub(crate) fn key_origin(key: PatchKey, terrain_span: f64, world: [f32; 2]) -> [f32; 2] {
    let span = key_span(key, terrain_span) as f32;
    [
        key[0] as f32 * span - world[0] * 0.5,
        key[1] as f32 * span - world[1] * 0.5,
    ]
}

/// `Vector2.distance_to`.
pub(crate) fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    distance_squared(a, b).sqrt()
}

/// `Vector2.distance_squared_to`.
pub(crate) fn distance_squared(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = a[0] - b[0];
    let dz = a[1] - b[1];
    dx * dx + dz * dz
}

/// The distance from the camera to the centre of `key` (`_patch_distance`), widened.
pub(crate) fn key_distance(
    key: PatchKey,
    terrain_span: f64,
    world: [f32; 2],
    camera: [f32; 2],
) -> f64 {
    f64::from(distance(key_centre(key, terrain_span, world), camera))
}

/// The camera's cell on the fine grid (`Vector2i((camera_xz / span).floor())`).
pub(crate) fn camera_cell(camera: [f32; 2], span: f64) -> Cell {
    let span = span as f32;
    [
        (camera[0] / span).floor() as i32,
        (camera[1] / span).floor() as i32,
    ]
}

/// The terrain patch that owns `key` (`_owner_key`): staleness is answered there.
pub(crate) fn owner(key: PatchKey) -> Cell {
    let divisor = key[2].max(1);
    [key[0].div_euclid(divisor), key[1].div_euclid(divisor)]
}

fn hysteresis(previous: bool) -> f64 {
    if previous { BAND_HYSTERESIS_M } else { 0.0 }
}

impl ResidencySettings {
    fn understory_end(&self, span: f64) -> f64 {
        let p = &self.product;
        p.bush_range_m.max(p.rock_range_m) + span * HALF_DIAGONAL
    }

    /// `_understory_wanted`: whether the patch is close enough to be worth the dense understory.
    /// Conservative by the patch half-diagonal, like the near band: a patch centre outside the
    /// longest understory range can still have a near corner inside it. On the terrain grid that
    /// slack was 361 m on a 420 m range, so nearly half of every understory patch generated was
    /// never drawn. Existing content is kept for the hysteresis past the entry range.
    pub(crate) fn understory(&self, distance: f64, span: f64, previous: bool) -> bool {
        distance >= 0.0 && distance <= self.understory_end(span) + hysteresis(previous)
    }

    /// `_near_band_wanted`: whether any part of the patch can fall inside the near band, and so
    /// whether its near per-variant meshes are built at all. Beyond it the patch builds one
    /// distant instance per canopy species. Existing near meshes persist through the hysteresis
    /// of retreat; entry is unchanged. Without a camera (a negative distance) every level is built.
    pub(crate) fn near_band(&self, distance: f64, span: f64, previous: bool) -> bool {
        distance < 0.0
            || distance <= self.product.near_m + span * HALF_DIAGONAL + hysteresis(previous)
    }

    /// `_shadow_caster_wanted`, from the patch's nearest corner: a patch with any part inside the
    /// sun's range casts whole (one instance cannot split), and keeps casting through the
    /// hysteresis; past the range it cannot reach a cascade. Independent of the runtime shadow
    /// toggle, which must turn casting back on without a rebuild.
    pub(crate) fn caster(&self, distance: f64, span: f64, previous: bool) -> bool {
        let reach = distance - span * HALF_DIAGONAL;
        distance < 0.0 || reach <= self.shadow_max_m + hysteresis(previous)
    }

    /// The bands at `distance`, each kept through its hysteresis from `previous` (`_bands_for`).
    pub(crate) fn bands(&self, distance: f64, span: f64, previous: Option<Bands>) -> Bands {
        let previous = previous.unwrap_or_default();
        Bands {
            understory: self.understory(distance, span, previous.understory),
            near_band: self.near_band(distance, span, previous.near_band),
            caster: self.caster(distance, span, previous.caster),
        }
    }

    /// The distance the camera must travel before `bands(_, span, Some(held))` can differ from
    /// what it is at `distance`: the nearest band boundary, less the slack. Each rule is a
    /// threshold on one distance that moves no faster than the camera.
    pub(crate) fn band_margin(&self, distance: f64, span: f64, held: Bands) -> f64 {
        let understory = (distance - (self.understory_end(span) + hysteresis(held.understory))).abs();
        let near = (distance
            - (self.product.near_m + span * HALF_DIAGONAL + hysteresis(held.near_band)))
        .abs();
        let caster = ((distance - span * HALF_DIAGONAL)
            - (self.shadow_max_m + hysteresis(held.caster)))
        .abs();
        understory.min(near).min(caster) - MARGIN_SLACK_M
    }
}
