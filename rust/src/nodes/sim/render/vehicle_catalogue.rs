// SPDX-License-Identifier: GPL-3.0-or-later

//! The vehicle catalogue (`RENDER-65`, docs/vehicles.md "Renderer"): the `[vehicle]` assets of
//! the loaded packs as the renderer draws them. Presentation only: the simulation never reads it,
//! and nothing of it is saved.
//!
//! - **Models** are sorted by qualified asset id, and each **look** is one model in one albedo
//!   variant. A car's drawn bucket is `look * 4 + tier`, so a `u8` key holds [`MAX_LOOKS`].
//! - **Which look and paint a vehicle gets** is a stable hash of its identity inside its body
//!   class ([`VehicleCatalogue::pick`]): the simulation's vehicle type says only car or freight.
//!   Moving and parked cars call the same function with the same identity, so a car keeps its
//!   look across the handoff. Another pack reshuffles the looks and changes no simulation result.
//! - **Tiers** switch at the asset's `[[lods]]` starts, read once here; the snapshot compares a
//!   car's squared distance to them, one comparison chain per posed car ([`Self::tier`]).
//! - **The runtime frame**: cars point their local `-Z` along travel, so every model is turned
//!   half a turn about `Y` at load (`x` and `z` negated); the facts handed out here are in that
//!   turned frame. The contract's frame has the nose at `+Z`.
//!
//! Built when packs load: O(vehicle assets × (wheels + lamps + paints)) plus O(assets log assets)
//! for the order. A pick is O(models of the class + paints) with no allocation.

use crate::assets::{AssetRegistry, BodyClass, LampGroup, VehicleData, VehicleLamp};
use crate::nodes::sim::render::vehicle_ground::VehicleGroundSupport;
use godot::prelude::Vector3;
use std::path::Path;

/// Looks a `u8` bucket key holds at four tiers each.
pub(crate) const MAX_LOOKS: usize = 64;
/// Tiers a model has.
pub(crate) const TIERS: usize = crate::assets::VEHICLE_TIERS;
/// The instance word's lamp bit for "lights on" (driving): the shader lights tail, low beam and
/// marker lenses by it at night. The brake, indicator and reverse bits are
/// [`super::vehicle_signals`]'s (`RENDER-66`); high beam and beacon stay zero (`RENDER-67`).
pub(crate) const LAMP_LIGHTS_ON: u32 = 1;

/// One drawable model.
pub(crate) struct CatalogueModel {
    /// `pack_id:asset_id`.
    pub(crate) id: String,
    /// The asset's folder.
    pub(crate) directory: String,
    /// Length, width and height, metres.
    pub(crate) size: [f32; 3],
    /// Tier files, relative to the folder, `LOD0` first.
    pub(crate) tiers: [String; TIERS],
    // Squared tier starts from LOD1 on, metres².
    tier_starts_sq: [f32; TIERS - 1],
    /// Tier starts, metres, `LOD0` (0) first.
    pub(crate) tier_starts_m: [f32; TIERS],
    /// The asset's facts (wheels and lenses in the contract frame).
    pub(crate) data: VehicleData,
    /// Palette in linear colour, at most 64.
    pub(crate) palette: Vec<[f32; 3]>,
    /// The steering and the fixed axle along the nose (`RENDER-66`), `None` without both.
    pub(crate) axles: Option<Axles>,
    /// The radius the wheels roll with, metres: the first wheel's (the pilot pair's wheels are
    /// all one size); 0 without wheels.
    pub(crate) wheel_radius_m: f32,
    // Palette weights.
    paint: Weights,
    // The number of albedo variants.
    variants: Divisor,
    first_look: u8,
}

/// Where a model's axles sit along its length, metres from the body centre toward the nose
/// (the contract frame's `z`): the mean of the steering wheels' centres and of the others'.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Axles {
    pub(crate) front_m: f32,
    pub(crate) rear_m: f32,
}

impl Axles {
    // From a model's wheels; `None` unless some steer and some do not, front ahead of rear.
    fn of(wheels: &[crate::assets::VehicleWheel]) -> Option<Self> {
        let mean = |steer: bool| {
            let (sum, count) = wheels
                .iter()
                .filter(|wheel| wheel.steer == steer)
                .fold((0.0, 0), |(sum, count), wheel| (sum + wheel.centre[2], count + 1));
            (count > 0).then(|| sum / count as f32)
        };
        let (front_m, rear_m) = (mean(true)?, mean(false)?);
        (front_m > rear_m).then_some(Self { front_m, rear_m })
    }

    /// Front axle to rear axle, metres.
    pub(crate) fn wheelbase_m(self) -> f32 {
        self.front_m - self.rear_m
    }
}

/// One model in one albedo variant.
pub(crate) struct Look {
    pub(crate) model: u8,
    pub(crate) variant: u8,
}

/// What a vehicle is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DrawnVehicle {
    pub(crate) look: u8,
    pub(crate) model: u8,
    pub(crate) paint: u8,
    /// The indicator blink's phase offset, 0 to 15.
    pub(crate) blink: u8,
}

impl DrawnVehicle {
    /// The instance custom data's packed word (`custom.z`): 8 lamp bits, 6 paint bits, 4 blink
    /// bits, 18 bits in all, exact in an `f32`.
    pub(crate) fn word(self, lamps: u32) -> f32 {
        (lamps & 0xff | u32::from(self.paint) << 8 | u32::from(self.blink) << 14) as f32
    }

    /// The instance's custom data as the snapshot gives it: the front wheels' steer target
    /// (`RENDER-66`), a zero the frame job replaces with the wheels' roll, the word, a spare.
    pub(crate) fn custom(self, steer: f32, lamps: u32) -> [f32; 4] {
        [steer, 0.0, self.word(lamps), 0.0]
    }
}

/// Weights are kept as whole parts of this, so a pick is exact integer arithmetic.
const WEIGHT_SCALE: f64 = 1_000_000.0;

/// The drawable models and looks of the loaded packs.
#[derive(Default)]
pub(crate) struct VehicleCatalogue {
    /// Counts catalogue rebuilds: a renderer whose meshes are of another revision rebuilds them.
    pub(crate) revision: u64,
    pub(crate) models: Vec<CatalogueModel>,
    pub(crate) looks: Vec<Look>,
    // Per body class (index `BodyClass as usize`): spawn weights and model indices.
    classes: [ClassModels; 4],
}

// A body class's models and their spawn weights.
#[derive(Default)]
struct ClassModels {
    weights: Weights,
    models: Vec<u8>,
}

// Weights as cumulative whole parts of `WEIGHT_SCALE`, with their total as a `Divisor`.
#[derive(Default)]
struct Weights {
    cumulative: Vec<u64>,
    total: Divisor,
}

impl Weights {
    fn push(&mut self, weight: f32) {
        let sum = self.cumulative.last().map_or(0, |&sum| sum) + weight_parts(weight);
        self.cumulative.push(sum);
        self.total = Divisor::new(sum);
    }

    // The index whose cumulative range holds `hash` modulo the total. O(entries), a handful.
    fn pick(&self, hash: u64) -> usize {
        let point = self.total.rem(hash);
        self.cumulative
            .iter()
            .take_while(|&&sum| sum <= point)
            .count()
    }
}

// `x % d` for a divisor fixed when packs load, exactly, without a hardware division: a pick runs
// for every drawn and every rebuilt parked car each frame. With `m = (2^64 - 1) / d` the
// quotient estimate `x m / 2^64` is never above `x / d` and less than three below it, so the
// remainder needs at most two corrections.
#[derive(Clone, Copy, Debug)]
struct Divisor {
    d: u64,
    m: u64,
}

impl Divisor {
    // The divisor `d`, 1 for 0.
    fn new(d: u64) -> Self {
        let d = d.max(1);
        Self { d, m: u64::MAX / d }
    }

    // `x % d`. O(1).
    fn rem(self, x: u64) -> u64 {
        let q = ((u128::from(x) * u128::from(self.m)) >> 64) as u64;
        let mut r = x - q * self.d;
        while r >= self.d {
            r -= self.d;
        }
        r
    }
}

impl Default for Divisor {
    fn default() -> Self {
        Self::new(1)
    }
}

impl VehicleCatalogue {
    /// The catalogue of every vehicle asset in `registry`, by qualified id. A model whose looks
    /// would pass [`MAX_LOOKS`] is left out (and logged).
    pub(crate) fn from_registry(registry: &AssetRegistry) -> Self {
        let mut ids: Vec<&str> = registry
            .qualified_ids()
            .filter(|id| registry.get(id).is_some_and(|e| e.manifest.vehicle.is_some()))
            .collect();
        ids.sort_unstable();
        let mut catalogue = Self::default();
        for id in ids {
            let entry = registry.get(id).expect("listed above");
            let data = entry.manifest.vehicle.clone().expect("a vehicle");
            if catalogue.looks.len() + data.variants.len() > MAX_LOOKS {
                crate::debug_log!("init", "vehicle {id} left out: more than {MAX_LOOKS} looks");
                continue;
            }
            let model = catalogue.models.len() as u8;
            let lods = &entry.manifest.lods;
            let tiers = std::array::from_fn(|tier| lods[tier].file.clone());
            let tier_starts_m: [f32; TIERS] = std::array::from_fn(|tier| lods[tier].distance_min_m);
            let first_look = catalogue.looks.len() as u8;
            for variant in 0..data.variants.len() {
                catalogue.looks.push(Look {
                    model,
                    variant: variant as u8,
                });
            }
            let palette = if data.paints.is_empty() {
                default_palette(data.body_class)
            } else {
                data.paints.iter().map(|p| (p.colour, p.weight)).collect()
            };
            let class = &mut catalogue.classes[data.body_class as usize];
            class.weights.push(data.spawn_weight);
            class.models.push(model);
            let mut paint = Weights::default();
            palette.iter().for_each(|&(_, weight)| paint.push(weight));
            catalogue.models.push(CatalogueModel {
                id: id.to_owned(),
                directory: entry.asset_dir.clone(),
                size: [data.length_m, data.width_m, data.height_m],
                tiers,
                tier_starts_sq: std::array::from_fn(|t| tier_starts_m[t + 1] * tier_starts_m[t + 1]),
                tier_starts_m,
                paint,
                variants: Divisor::new(data.variants.len() as u64),
                palette: palette.iter().map(|&(c, _)| c.map(srgb_to_linear)).collect(),
                axles: Axles::of(&data.wheels),
                wheel_radius_m: data.wheels.first().map_or(0.0, |wheel| wheel.radius_m),
                data,
                first_look,
            });
        }
        catalogue
    }

    /// The look and paint of a vehicle: `freight` draws from the truck class (then van, then
    /// car, whichever has models), any other from the car class. `None` with no model at all.
    pub(crate) fn pick(&self, freight: bool, identity: u64) -> Option<DrawnVehicle> {
        let hash = mix(identity);
        let model = self.pick_model(freight, hash)?;
        let entry = &self.models[usize::from(model)];
        let variant = entry.variants.rem(hash >> 24) as u8;
        // The paint from a second mix, so it does not follow the model's or variant's pick.
        let paint = entry.paint.pick(mix(hash)) as u8;
        Some(DrawnVehicle {
            look: entry.first_look + variant,
            model,
            paint,
            blink: (hash >> 60) as u8,
        })
    }

    /// The model [`Self::pick`] gives a vehicle, alone (a parked car's ground support). O(models
    /// of the class).
    pub(crate) fn model(&self, freight: bool, identity: u64) -> Option<u8> {
        self.pick_model(freight, mix(identity))
    }

    // The model of the class `freight` draws from, by the identity's first mix.
    fn pick_model(&self, freight: bool, hash: u64) -> Option<u8> {
        let order: &[BodyClass] = if freight {
            &[BodyClass::Truck, BodyClass::Van, BodyClass::Car]
        } else {
            &[BodyClass::Car, BodyClass::Van, BodyClass::Truck]
        };
        let class = order
            .iter()
            .map(|&class| &self.classes[class as usize])
            .find(|class| !class.models.is_empty())?;
        Some(class.models[class.weights.pick(hash)])
    }

    /// The tier of a car of `model` at squared distance `distance_sq` from the eye.
    pub(crate) fn tier(&self, model: u8, distance_sq: f32) -> u8 {
        self.models
            .get(usize::from(model))
            .map_or(0, |m| m.tier_starts_sq.iter().filter(|&&start| distance_sq >= start).count() as u8)
    }

    /// Fills `radii` with each bucket key's wheel radius, metres (`RENDER-66`): O(looks × tiers),
    /// at most 256, with no allocation once `radii` has grown.
    pub(crate) fn fill_wheel_radii(&self, radii: &mut Vec<f32>) {
        radii.clear();
        radii.extend(self.looks.iter().flat_map(|look| {
            [self.models[usize::from(look.model)].wheel_radius_m; TIERS]
        }));
    }

    /// Each model's ground support from its wheel facts (`RENDER-65`): the plan box and the
    /// a 3 × 3 grid over every wheel's contact patch (both tread edges and the middle, under the
    /// centre and a quarter radius ahead and behind, which covers the flat bottom of a 16-sided
    /// tyre and the gap of a twin), in the turned frame. With no
    /// model, the procedural fallback car's.
    pub(crate) fn ground_supports(&self) -> Vec<VehicleGroundSupport> {
        if self.models.is_empty() {
            return vec![VehicleGroundSupport::default()];
        }
        self.models
            .iter()
            .map(|model| {
                let [length, width, _] = model.size;
                let (x, z) = (width * 0.5, length * 0.5);
                let contacts: Vec<Vector3> = model
                    .data
                    .wheels
                    .iter()
                    .flat_map(|wheel| {
                        let [cx, _, cz] = turned(wheel.centre);
                        let patch = 0.25 * wheel.radius_m;
                        [-1.0, 0.0, 1.0]
                            .into_iter()
                            .flat_map(|end| [-0.5, 0.0, 0.5].map(|side| (side, end)))
                            .map(move |(side, end)| {
                            Vector3::new(
                                (cx + side * wheel.width_m).clamp(-x, x),
                                0.0,
                                (cz + end * patch).clamp(-z, z),
                            )
                        })
                    })
                    .collect();
                VehicleGroundSupport::new([-x, -z, x, z], &contacts).unwrap_or_default()
            })
            .collect()
    }
}

impl CatalogueModel {
    /// The right-hand lens of `groups` (the first group that has one) facing most along
    /// `forward` (+1 the nose, -1 the tail), in the turned frame. Found by group and facing:
    /// a lens's `position` alone does not say a wrap-round lamp faces forward.
    pub(crate) fn lamp(&self, groups: &[LampGroup], forward: f32) -> Option<Vector3> {
        groups.iter().find_map(|&group| {
            self.lenses(group)
                .filter(|lamp| lamp.centre[0] < 0.0)
                .max_by(|a, b| (a.facing[2] * forward).total_cmp(&(b.facing[2] * forward)))
                .filter(|lamp| lamp.facing[2] * forward > 0.0)
                .map(|lamp| Vector3::from_array(turned(lamp.centre)))
        })
    }

    fn lenses(&self, group: LampGroup) -> impl Iterator<Item = &VehicleLamp> {
        self.data.lamps.iter().filter(move |lamp| lamp.group == group)
    }

    /// The asset folder's file `relative`.
    pub(crate) fn file(&self, relative: &str) -> String {
        Path::new(&self.directory).join(relative).to_string_lossy().into_owned()
    }
}

/// A contract-frame point in the runtime's turned frame (half a turn about `Y`).
pub(crate) fn turned([x, y, z]: [f32; 3]) -> [f32; 3] {
    [-x, y, -z]
}

// splitmix64's finaliser: identities that differ in one bit pick independently.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn weight_parts(weight: f32) -> u64 {
    ((f64::from(weight) * WEIGHT_SCALE).round() as u64).max(1)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

// The palette a model gets when its asset lists none (sRGB, weight): car colours by rough share
// on Nordic roads; trucks mostly white fleet bodies.
fn default_palette(class: BodyClass) -> Vec<([f32; 3], f32)> {
    match class {
        BodyClass::Car | BodyClass::Van => vec![
            ([0.92, 0.92, 0.91], 24.0),
            ([0.06, 0.06, 0.07], 18.0),
            ([0.26, 0.27, 0.29], 14.0),
            ([0.63, 0.65, 0.67], 14.0),
            ([0.12, 0.24, 0.47], 10.0),
            ([0.55, 0.07, 0.07], 7.0),
            ([0.10, 0.26, 0.18], 4.0),
            ([0.71, 0.63, 0.50], 3.0),
            ([0.45, 0.58, 0.72], 3.0),
            ([0.31, 0.21, 0.14], 2.0),
            ([0.85, 0.55, 0.10], 1.0),
        ],
        BodyClass::Truck | BodyClass::Bus => vec![
            ([0.92, 0.92, 0.91], 50.0),
            ([0.62, 0.08, 0.07], 12.0),
            ([0.10, 0.22, 0.48], 12.0),
            ([0.24, 0.25, 0.27], 10.0),
            ([0.90, 0.70, 0.10], 8.0),
            ([0.12, 0.34, 0.20], 8.0),
        ],
    }
}

#[cfg(test)]
mod tests;

use crate::nodes::sim::render::vehicle_signals::CarPath;

impl crate::nodes::sim::core::SimCore {
    /// Replaces the catalogue from the asset registry and the ground supports with its models'
    /// (each parked pose follows). O(vehicle assets + bays), when packs load.
    pub(crate) fn rebuild_vehicle_catalogue(&mut self) {
        let revision = self.vehicle_catalogue.revision + 1;
        self.vehicle_catalogue = VehicleCatalogue::from_registry(&self.allocator.registry);
        self.vehicle_catalogue.revision = revision;
        self.vehicle_ground_support = self.vehicle_catalogue.ground_supports();
        self.parking.invalidate_all_ground();
        self.vehicle_ground_generation += 1;
    }

    /// Agent `i`'s car bucket (`look * 4 + tier`) and instance custom data, the tier for a squared
    /// distance `distance_sq` to the eye (`None`: the nearest tier), its lamps and steer target
    /// from `path`, how the snapshot posed it (`RENDER-66`, [`SimCore::car_signals`]). A car with
    /// a parking vehicle is named by it, as its parked pose is, so it keeps its look across the
    /// handoff; any other by its render id. Bucket 0 with no model at all (the procedural
    /// fallback). O(1) plus the signals' lane samples.
    pub(crate) fn drawn_car(
        &self,
        i: usize,
        distance_sq: Option<f32>,
        path: CarPath,
    ) -> (u8, [f32; 4]) {
        use crate::simulation::economy::agents::VEHICLE_FREIGHT_DELIVERY;
        // A drop-off car is not the household car its row names (`PARK-34`).
        let trip = &self.agents.parking_trip[i];
        let identity = self
            .parking
            .vehicle_visual(trip.vehicle)
            .filter(|_| !trip.drop_off)
            .map_or(self.agents.render_id[i], |(_, _, identity)| identity);
        let freight = self.agents.vehicle_type[i] == VEHICLE_FREIGHT_DELIVERY;
        let Some(drawn) = self.vehicle_catalogue.pick(freight, identity) else {
            let (lamps, _) = self.car_signals(i, None, 0, distance_sq, path);
            return (0, [0.0, 0.0, lamps as f32, 0.0]);
        };
        let tier = distance_sq.map_or(0, |d| self.vehicle_catalogue.tier(drawn.model, d));
        let axles = self.vehicle_catalogue.models[usize::from(drawn.model)].axles;
        let (lamps, steer) = self.car_signals(i, axles, tier, distance_sq, path);
        (drawn.look * TIERS as u8 + tier, drawn.custom(steer, lamps))
    }

    /// The model (ground support profile) bucket `key` draws. O(1).
    pub(crate) fn vehicle_bucket_model(&self, key: u8) -> u8 {
        self.vehicle_catalogue
            .looks
            .get(usize::from(key) / TIERS)
            .map_or(0, |look| look.model)
    }
}

/// The catalogue as the parked chunks ask it (`RENDER-65`): a parked car is drawn as the look its
/// identity picks, posed by that look's model, with every lamp bit zero.
pub(crate) struct CatalogueLooks<'a>(pub(crate) &'a VehicleCatalogue);

impl crate::simulation::parking::ParkedLooks for CatalogueLooks<'_> {
    fn look(&self, model: u8, _colour: u8, identity: u64) -> (u8, u8, [f32; 4]) {
        let freight = model == crate::simulation::economy::agents::VEHICLE_FREIGHT_DELIVERY;
        self.0
            .pick(freight, identity)
            .map_or((0, 0, [0.0; 4]), |drawn| (drawn.look, drawn.model, drawn.custom(0.0, 0)))
    }

    fn ground_model(&self, model: u8, _colour: u8, identity: u64) -> u8 {
        let freight = model == crate::simulation::economy::agents::VEHICLE_FREIGHT_DELIVERY;
        self.0.model(freight, identity).unwrap_or(0)
    }
}
