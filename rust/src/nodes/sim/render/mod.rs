// SPDX-License-Identifier: GPL-3.0-or-later

//! Specialized rendering bridge sub-modules (Item R12).
//!
//! Decouples Godot-facing render transform generation into focused units
//! for agents, buildings, road network, and zoning overlays.

/// The agents' frame prepared a frame ahead on the background pool (`RENDER-47`).
pub(crate) mod agent_frame;
pub(crate) mod agent_inspection;
/// Agent-specific rendering (pedestrians, cars, path debug).
pub mod agents;
pub(crate) mod building_lod;
/// The construction sites' rows, published with the per-frame values (`RENDER-30`).
pub(crate) mod building_sites;
/// Building-specific rendering (asset transforms, construction sites).
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
pub(crate) mod vehicle_catalogue;
pub(crate) mod vehicle_ground;
/// Brake lamps, indicators, the reverse lamp and the steered wheels (`RENDER-66`).
pub(crate) mod vehicle_signals;
/// Street lights' lamp grid, far light map, light pool and chunk batches (`RENDER-32-H`).
pub(crate) mod street_lights;
/// The streaming core: render-consumer state chosen from the view (`RENDER-32`).
pub(crate) mod stream;
/// A vegetation patch's instance buffers, bounds and ranges (`RENDER-32-C`).
pub(crate) mod vegetation_product;
/// The walkers' drawn buffers between snapshots, uploaded only when changed (`RENDER-28`).
pub(crate) mod walker_visual;
/// Water patch mesh generation and cache helpers.
pub mod water;
/// The water a view ray first comes down onto, for the planar reflection's mirror planes.
pub(crate) mod water_reflection;
/// Zoning and environment rendering (image overlays).
pub mod zoning;
pub(crate) mod zoning_mask;
