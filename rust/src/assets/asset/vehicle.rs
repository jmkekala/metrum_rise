//! Vehicle asset schema.

use serde::Deserialize;

// ── Vehicle ──────────────────────────────────���────────────────────────────────

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

/// Physical form factor of a vehicle asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleFamily {
    /// Passenger car (two-box or three-box body).
    Sedan,
    /// Sport-utility vehicle or crossover.
    Suv,
    /// Van or minivan.
    Van,
    /// Truck, lorry, or heavy goods vehicle.
    Truck,
}

/// One colour or livery variant for a vehicle asset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorVariant {
    /// Short display name for this variant (e.g. `"red"`, `"police_livery"`).
    pub name: String,
    /// Path to the albedo texture for this variant, relative to the asset folder.
    pub albedo_file: String,
}

/// Class-specific data for a vehicle asset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleData {
    /// Gameplay category (determines spawning rules and icon).
    pub vehicle_class: VehicleClass,
    /// Physical form factor (determines collision and lane-fit checks).
    pub vehicle_family: VehicleFamily,
    /// Vehicle length in metres (bumper to bumper).
    pub length_m: f32,
    /// Vehicle width in metres (mirror to mirror).
    pub width_m: f32,
    /// Vehicle height in metres (ground to roof).
    pub height_m: f32,
    /// Available colour or livery variants. At least one must be provided.
    #[serde(default)]
    pub color_variants: Vec<ColorVariant>,
}
