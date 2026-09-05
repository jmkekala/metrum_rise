// SPDX-License-Identifier: GPL-2.0-only

//! Agent movement, routing, traffic, and congestion tests.

use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, BuildingEntrance};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use godot::prelude::{Vector2, Vector3};

mod junctions;
mod lane_dynamics;
mod support;
mod trips;
mod vehicle_flow;
