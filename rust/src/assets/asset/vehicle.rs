// SPDX-License-Identifier: GPL-3.0-or-later

//! Vehicle asset schema (`TOOLS-41`, docs/vehicles.md "The `[vehicle]` pack class"): the model
//! contract's facts a pack asset carries, copied from the facts sidecar by the vehicle draft,
//! and the checks a pack load makes on them. O(wheels + lamps + paints) per asset, at load only.

use crate::assets::ManifestError;
use serde::Deserialize;

/// Broad gameplay category for a vehicle asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleClass {
    /// Ordinary civilian traffic.
    Civil,
    /// Police / law-enforcement.
    Police,
    /// Fire / emergency.
    Fire,
    /// Ambulance / medical.
    Ambulance,
    /// Maintenance, utility, or delivery.
    Utility,
    /// Public transit bus.
    Bus,
}

/// Physical form factor of a vehicle asset. Nothing in the simulation reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleFamily {
    /// Passenger car with a separate boot.
    Sedan,
    /// Passenger car with a hatch.
    Hatchback,
    /// Passenger car with a long roof and a tailgate.
    Estate,
    /// Sport-utility vehicle or crossover.
    Suv,
    /// Van or minivan.
    Van,
    /// Truck, lorry, or heavy goods vehicle.
    Truck,
    /// Bus.
    Bus,
}

/// The size class a vehicle is parked and queued as (docs/vehicles.md "Body classes"): every
/// model of a class must fit the class box, so models never change simulation results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyClass {
    /// 4.50 × 1.80 × 1.95 m.
    Car,
    /// 6.00 × 2.10 × 2.70 m.
    Van,
    /// 8.00 × 2.55 × 3.80 m.
    Truck,
    /// 12.00 × 2.55 × 3.20 m.
    Bus,
}

impl BodyClass {
    /// The class box, length × width (without mirrors) × height, metres.
    pub fn box_m(self) -> [f32; 3] {
        match self {
            Self::Car => [4.50, 1.80, 1.95],
            Self::Van => [6.00, 2.10, 2.70],
            Self::Truck => [8.00, 2.55, 3.80],
            Self::Bus => [12.00, 2.55, 3.20],
        }
    }
}

/// A wheel's side, or a lamp's: always the driver's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The driver's left, `+X` in the model frame.
    Left,
    /// The driver's right, `-X`.
    Right,
    /// On the centre line (a single lens).
    Centre,
}

/// The side a door opens on (docs/vehicles.md "Doors"): the driver's left or right, or the rear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoorSide {
    /// The driver's left, `+X` in the model frame.
    Left,
    /// The driver's right, `-X`.
    Right,
    /// The rear plane (a tailgate or rear load door).
    Rear,
}

/// The row a door serves: listed front row first, then rear row, then cargo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoorRow {
    /// The driver's and front passenger's doors.
    Front,
    /// Rear passenger doors.
    Rear,
    /// A load door or tailgate, on the rear or a side; no person uses it.
    Cargo,
}

/// One door (`[[vehicle.doors]]`, sidecar version 3, `TOOLS-43`): a painted seam of the body.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleDoor {
    /// The side it opens on.
    pub side: DoorSide,
    /// The row it serves.
    pub row: DoorRow,
    /// The opening's centre on the door skin at ground level (`y = 0`), model frame, metres. The
    /// game adds its own outward offset for where a person stands.
    pub centre: [f32; 3],
    /// The opening's width along the body, metres.
    pub width_m: f32,
}

/// Lamp groups and their vertex surface ids (docs/vehicles.md "Lamps").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LampGroup {
    /// 1: rear position lamp, lit with the lights.
    Tail,
    /// 2: brake lamp.
    Brake,
    /// 3: a rear lens that is both tail and brake.
    TailBrake,
    /// 4: left indicator.
    IndicatorLeft,
    /// 5: right indicator.
    IndicatorRight,
    /// 6: reversing lamp.
    Reverse,
    /// 7: dipped headlamp.
    LowBeam,
    /// 8: main beam.
    HighBeam,
    /// 9: daytime running lamp.
    Daytime,
    /// 10: front fog lamp (reserved).
    FogFront,
    /// 11: rear fog lamp (reserved).
    FogRear,
    /// 12: plate lamp (reserved).
    Plate,
    /// 13: roof beacon (reserved).
    Beacon,
    /// 14: taxi sign or destination board (reserved).
    Sign,
    /// 15: outline marker of a truck or bus (reserved).
    Marker,
}

impl LampGroup {
    /// The group's surface id, the `G` byte of its faces.
    pub fn id(self) -> u8 {
        self as u8 + 1
    }
}

/// Where a lens faces, from its facing's main axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LampPosition {
    /// Faces forward.
    Front,
    /// Faces backward.
    Rear,
    /// Faces sideways.
    Side,
    /// Faces up.
    Roof,
}

/// A lens's lit colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LampColour {
    /// White.
    White,
    /// Amber.
    Amber,
    /// Red.
    Red,
}

/// One wheel part (`[[vehicle.wheels]]`), in sidecar order: front axle first, left first.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleWheel {
    /// The wheel's part id, the `R` byte of its vertices (1 to 8).
    pub part: u8,
    /// The driver's side.
    pub side: Side,
    /// Wheel centre in the model frame (`+Y` up, nose at `+Z`), metres; `y` equals the radius.
    pub centre: [f32; 3],
    /// Radius, metres.
    pub radius_m: f32,
    /// Tread width, metres (both tyres of a twin wheel).
    pub width_m: f32,
    /// Whether the game steers it about `Y` through its centre.
    pub steer: bool,
    /// Twin tyres on one part.
    #[serde(default)]
    pub twin: bool,
}

/// One lens (`[[vehicle.lamps]]`), a lamp group's connected faces measured from `LOD0`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleLamp {
    /// The lamp group.
    pub group: LampGroup,
    /// The group's id, repeated as the sidecar states it; must match `group`.
    pub lamp: u8,
    /// The driver's side, or the centre line.
    pub side: Side,
    /// Which way the lens faces.
    pub position: LampPosition,
    /// The lit colour.
    pub colour: LampColour,
    /// Lens centre in the model frame, metres.
    pub centre: [f32; 3],
    /// Unit outward normal of the lens surface: not a beam direction.
    pub facing: [f32; 3],
    /// Lens size along up × facing, metres.
    pub width_m: f32,
    /// Lens size across it, metres.
    pub height_m: f32,
}

/// One albedo variant (`[[vehicle.variants]]`): another albedo on the same geometry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleVariant {
    /// Variant name, lowercase letters, digits and underscores (`plain`, `haulier`).
    pub name: String,
    /// The albedo texture, relative to the asset folder.
    pub albedo: String,
    /// Whether painted faces take the vehicle's paint colour; a livery that paints its panels
    /// itself is `false` and is drawn with white paint.
    pub paint: bool,
}

/// One paint of the palette (`[[vehicle.paints]]`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehiclePaint {
    /// sRGB colour, each channel 0 to 1.
    pub colour: [f32; 3],
    /// Share among the paints; positive.
    pub weight: f32,
}

/// Most paints a model may list: the paint index is six bits of the instance's look word.
pub const MAX_PAINTS: usize = 64;
/// Most wheel parts a model may have.
pub const MAX_WHEELS: usize = 8;
/// Tiers every vehicle has (`LOD0` to `LOD3`).
pub const VEHICLE_TIERS: usize = 4;
/// What a dimension may exceed its class box by: rounding in the sidecar only.
const BOX_TOLERANCE_M: f32 = 0.005;
/// A wheel's centre height may differ from its radius by this much (contract: 1 mm).
const GROUND_TOLERANCE_M: f32 = 0.001;
/// Lamp groups every vehicle carries (besides tail and brake, checked apart).
const REQUIRED_LAMPS: [LampGroup; 6] = [
    LampGroup::IndicatorLeft,
    LampGroup::IndicatorRight,
    LampGroup::Reverse,
    LampGroup::LowBeam,
    LampGroup::HighBeam,
    LampGroup::Daytime,
];

/// Class-specific data for a vehicle asset (`[vehicle]`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleData {
    /// Gameplay category.
    pub vehicle_class: VehicleClass,
    /// Physical form factor; descriptive only.
    pub vehicle_family: VehicleFamily,
    /// The size class the simulation parks and queues it as.
    pub body_class: BodyClass,
    /// Body length bumper to bumper, metres.
    pub length_m: f32,
    /// Body width without mirrors, metres.
    pub width_m: f32,
    /// Height from the ground, metres.
    pub height_m: f32,
    /// Share of this model among the models of its body class; positive.
    #[serde(default = "unit_weight")]
    pub spawn_weight: f32,
    /// Seats, driver included: a value of the pack asset, not of the model.
    pub seats: u8,
    /// The occlusion-roughness-metallic map, relative to the asset folder.
    pub orm: String,
    /// The emission map: lit lenses and the cabin picture on glass.
    pub emission: String,
    /// The optional normal map.
    #[serde(default)]
    pub normal: Option<String>,
    /// Albedo variants; the first is the default.
    pub variants: Vec<VehicleVariant>,
    /// The paint palette; empty takes the body class's default palette.
    #[serde(default)]
    pub paints: Vec<VehiclePaint>,
    /// Wheel parts in part order.
    pub wheels: Vec<VehicleWheel>,
    /// Lenses, one entry each.
    pub lamps: Vec<VehicleLamp>,
    /// Doors in list order (front row, rear row, cargo; left before right); empty for a model
    /// drafted from a version 2 sidecar. The renderer and tools read them; the simulation keeps
    /// its own per-class door points (`simulation::parking::doors`).
    #[serde(default)]
    pub doors: Vec<VehicleDoor>,
}

fn unit_weight() -> f32 {
    1.0
}

impl VehicleData {
    /// Every texture the asset references, relative to its folder.
    pub fn textures(&self) -> impl Iterator<Item = &str> {
        [Some(self.orm.as_str()), Some(self.emission.as_str()), self.normal.as_deref()]
            .into_iter()
            .flatten()
            .chain(self.variants.iter().map(|v| v.albedo.as_str()))
    }

    /// The pack-load checks: the class box, wheels resting on the ground in part order, the
    /// required lamp groups, variants and paints. `lods` is the asset's `[[lods]]`.
    pub(crate) fn validate(&self, asset_id: &str, lods: usize) -> Result<(), ManifestError> {
        let fail = |message: String| {
            Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': vehicle {message}"
            )))
        };
        let size = [self.length_m, self.width_m, self.height_m];
        let limit = self.body_class.box_m();
        if size.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return fail("dimensions must be finite and positive".into());
        }
        if size.iter().zip(limit).any(|(v, l)| *v > l + BOX_TOLERANCE_M) {
            return fail(format!(
                "{:?} × {:?} × {:?} m does not fit the {:?} box {limit:?}",
                self.length_m, self.width_m, self.height_m, self.body_class
            ));
        }
        if lods != VEHICLE_TIERS {
            return fail(format!("needs {VEHICLE_TIERS} [[lods]], LOD0 to LOD3; has {lods}"));
        }
        if !(self.spawn_weight.is_finite() && self.spawn_weight > 0.0) {
            return fail("spawn_weight must be positive".into());
        }
        if self.seats == 0 {
            return fail("seats must be 1 or more".into());
        }
        self.validate_wheels().or_else(fail)?;
        self.validate_lamps().or_else(fail)?;
        self.validate_doors().or_else(fail)?;
        if self.variants.is_empty() {
            return fail("needs at least one [[vehicle.variants]] entry".into());
        }
        for (index, variant) in self.variants.iter().enumerate() {
            let name = &variant.name;
            if name.is_empty()
                || !name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
                || self.variants[..index].iter().any(|v| &v.name == name)
            {
                return fail(format!("variant name '{name}' is invalid or repeated"));
            }
        }
        if self.paints.len() > MAX_PAINTS {
            return fail(format!("lists {} paints; at most {MAX_PAINTS}", self.paints.len()));
        }
        if self.paints.iter().any(|paint| {
            !(paint.weight.is_finite() && paint.weight > 0.0)
                || paint.colour.iter().any(|c| !(0.0..=1.0).contains(c))
        }) {
            return fail("paints need channels 0 to 1 and a positive weight".into());
        }
        Ok(())
    }

    // At least four wheels, parts 1, 2, ... in order, each resting on the ground.
    fn validate_wheels(&self) -> Result<(), String> {
        if !(4..=MAX_WHEELS).contains(&self.wheels.len()) {
            return Err(format!("needs 4 to {MAX_WHEELS} wheels; has {}", self.wheels.len()));
        }
        for (index, wheel) in self.wheels.iter().enumerate() {
            if usize::from(wheel.part) != index + 1 {
                return Err(format!("wheel {index} must be part {}", index + 1));
            }
            if !(wheel.radius_m.is_finite() && wheel.radius_m > 0.0 && wheel.width_m > 0.0)
                || wheel.centre.iter().any(|v| !v.is_finite())
                || (wheel.centre[1] - wheel.radius_m).abs() > GROUND_TOLERANCE_M
            {
                return Err(format!(
                    "wheel part {} must rest on the ground: centre height equal to its radius",
                    wheel.part
                ));
            }
        }
        Ok(())
    }

    // Doors, when listed (docs/vehicles.md "Doors"): on the ground, in list order, a rear side only
    // for a cargo door, side doors on their own side, the front row on both sides (any one door
    // for a bus) and every row present on both sides mirrored within 1 mm.
    fn validate_doors(&self) -> Result<(), String> {
        if self.doors.is_empty() {
            return Ok(());
        }
        for door in &self.doors {
            let [x, y, _] = door.centre;
            if door.centre.iter().any(|v| !v.is_finite())
                || y.abs() > GROUND_TOLERANCE_M
                || !(door.width_m.is_finite() && door.width_m > 0.0)
            {
                return Err(format!(
                    "door {:?} {:?} needs a finite centre on the ground and a positive width",
                    door.side, door.row
                ));
            }
            let own_side = match door.side {
                DoorSide::Left => x > 0.0,
                DoorSide::Right => x < 0.0,
                DoorSide::Rear => door.row == DoorRow::Cargo,
            };
            if !own_side {
                return Err(format!(
                    "door {:?} {:?} is not on its side (left +X, right -X, rear only for cargo)",
                    door.side, door.row
                ));
            }
        }
        if self.doors.windows(2).any(|pair| (pair[0].row, pair[0].side) >= (pair[1].row, pair[1].side)) {
            return Err("doors must be listed front, rear, cargo, left before right, once each".into());
        }
        let find = |row: DoorRow, side: DoorSide| {
            self.doors.iter().find(|door| door.row == row && door.side == side)
        };
        let front = (find(DoorRow::Front, DoorSide::Left), find(DoorRow::Front, DoorSide::Right));
        if self.body_class != BodyClass::Bus && (front.0.is_none() || front.1.is_none()) {
            return Err("needs both front doors".into());
        }
        for row in [DoorRow::Front, DoorRow::Rear, DoorRow::Cargo] {
            if let (Some(left), Some(right)) = (find(row, DoorSide::Left), find(row, DoorSide::Right)) {
                let mirrored = [-right.centre[0], right.centre[1], right.centre[2]];
                if left.centre.iter().zip(mirrored).any(|(a, b)| (a - b).abs() > GROUND_TOLERANCE_M) {
                    return Err(format!("{row:?} doors must mirror in X within 1 mm"));
                }
            }
        }
        Ok(())
    }

    // Ids agree with groups; every lens is finite; the required groups are present.
    fn validate_lamps(&self) -> Result<(), String> {
        for lamp in &self.lamps {
            if lamp.lamp != lamp.group.id() {
                return Err(format!(
                    "lamp {:?} must have id {}, not {}",
                    lamp.group,
                    lamp.group.id(),
                    lamp.lamp
                ));
            }
            let [x, y, z] = lamp.facing;
            if lamp.centre.iter().chain(&lamp.facing).any(|v| !v.is_finite())
                || ((x * x + y * y + z * z).sqrt() - 1.0).abs() > 0.01
            {
                return Err(format!("lamp {:?} needs a finite centre and a unit facing", lamp.group));
            }
        }
        let has = |group: LampGroup| self.lamps.iter().any(|lamp| lamp.group == group);
        let rear = has(LampGroup::TailBrake) || (has(LampGroup::Tail) && has(LampGroup::Brake));
        if let Some(missing) = REQUIRED_LAMPS.into_iter().find(|&group| !has(group)) {
            return Err(format!("misses the required lamp group {missing:?}"));
        }
        if !rear {
            return Err("needs tail_brake lenses, or tail and brake lenses".into());
        }
        if self.body_class == BodyClass::Car && !has(LampGroup::Brake) {
            return Err("a car needs its high-mounted brake lamp (group brake)".into());
        }
        if self.body_class == BodyClass::Truck && !has(LampGroup::Marker) {
            return Err("a truck needs its marker lamps".into());
        }
        Ok(())
    }
}
