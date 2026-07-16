//! Building allocator placement, indexing, lifecycle, and demand tests.

use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::types::VehicleFrontageAccess;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoneType;
use godot::prelude::{Vector2, Vector3};
use rand::SeedableRng;

mod demand_actions;
mod demand_selection;
mod entrances;
mod indexing;
mod lifecycle;
mod placement;
mod runtime;
mod support;
