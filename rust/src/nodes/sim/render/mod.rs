// SPDX-License-Identifier: GPL-2.0-only

//! Specialized rendering bridge sub-modules (Item R12).
//!
//! Decouples Godot-facing render transform generation into focused units
//! for agents, buildings, road network, and zoning overlays.

pub(crate) mod agent_inspection;
/// Agent-specific rendering (pedestrians, cars, path debug).
pub mod agents;
pub(crate) mod building_lod;
/// Building-specific rendering (asset transforms, plots).
pub mod buildings;
pub(crate) mod car_visual;
pub(crate) mod junctions;
/// Shared lane pose sampling for render transforms.
pub mod lane_pose;
/// Morning-mist presence and water level over the world.
pub(crate) mod mist;
/// Road-network rendering (mesh data, connection helpers).
pub mod network;
/// Authored resource overlay rendering.
pub mod resources;
pub(crate) mod vehicle_ground;
/// Street lights' lamp grid, far light map, light pool and chunk batches (`RENDER-32-H`).
pub(crate) mod street_lights;
/// The streaming core: render-consumer state chosen from the view (`RENDER-32`).
pub(crate) mod stream;
/// A vegetation patch's instance buffers, bounds and ranges (`RENDER-32-C`).
pub(crate) mod vegetation_product;
/// Water patch mesh generation and cache helpers.
pub mod water;
/// The water a view ray first comes down onto, for the planar reflection's mirror planes.
pub(crate) mod water_reflection;
/// Zoning and environment rendering (image overlays).
pub mod zoning;
pub(crate) mod zoning_mask;
