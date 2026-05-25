//! Node-height field construction and evaluation tests.

use super::super::arrangement::NodeSeamSource;
use super::build::height_fields_by_source;
use super::grade::apply_junctionn_height_authority_normalization;
use super::model::*;
use super::seams::*;
use super::triangles::height_triangles_from_contour;
use super::vertices::height_vertex_heights_from_vertices;
use super::*;
use crate::simulation::network::surface::backend::road_points_to_polyline;
use crate::simulation::network::surface::input::NodeInputMouth;
use crate::simulation::network::surface::ownership::{
    NodeBooleanOwnership, NodeCarrierProvenanceClosure, NodeOwnedRegionArrangement,
};
use crate::simulation::network::surface::rails::NodeRailContourSet;
use crate::simulation::network::surface::terminal::{
    TerminalCapBandProvenance, TerminalCapBandRole,
};
use crate::simulation::network::surface::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
};
use godot::prelude::{Vector2, Vector3};
use std::collections::BTreeMap;

mod carriers;
mod generated_contours;
mod seams;
mod shared_vertices;
mod support;
mod terminal;

use support::*;
