// SPDX-License-Identifier: GPL-2.0-only

//! Specialized rendering bridge sub-modules (Item R12).
//!
//! Decouples Godot-facing render transform generation into focused units
//! for agents, buildings, road network, and zoning overlays.

/// Agent-specific rendering (pedestrians, cars, path debug).
pub mod agents;
/// Building-specific rendering (asset transforms, plots).
pub mod buildings;
/// Shared lane pose sampling for render transforms.
pub mod lane_pose;
/// Road-network rendering (mesh data, connection helpers).
pub mod network;
/// Authored resource overlay rendering.
pub mod resources;
/// Water patch mesh generation and cache helpers.
pub mod water;
/// Zoning and environment rendering (image overlays).
pub mod zoning;
