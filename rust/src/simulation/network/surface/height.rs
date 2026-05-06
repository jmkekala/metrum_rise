//! Spline-backed height evaluation for canonical node-owned regions.

#![allow(dead_code)]

use super::arrangement::{
    NodeBandHeightFieldId, NodeBandOwner, NodeHeightSource, NodeRegionSeamConstraint,
    seam_source_priority,
};
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2, RoadVec3, overlay_point_to_road,
    quantize_road_vec2_to_overlay_grid,
};
use super::input::{
    NodeArrangementInput, NodeInputBandInterval, NodeInputBoundaryRail, NodeInputBoundaryRailRole,
    NodeInputMouth, NodeInputTerminalEndBand,
};
use super::ownership::{NodeBooleanOwnedRegion, NodeBooleanOwnership};
use super::{
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind,
};
use splines::{Interpolation, Key, Spline};
use std::collections::BTreeMap;

const HEIGHT_KEY_SCALE: f64 = 1000.0;
const HEIGHT_PARAMETER_KEY_SCALE: f64 = 1_000_000.0;
const HEIGHT_PARAMETER_BOUNDARY_EPS: f64 = 32.0 / ROAD_OVERLAY_COORDINATE_SCALE;
const HEIGHT_PARAMETER_BOUNDARY_DISTANCE_EPS_M: f64 = 0.002;
const HEIGHT_FIELD_MIN_AXIS_LEN2_M2: f64 = 1.0e-12;
type NodeHeightedContour = Vec<NodeHeightedVertex>;
type NodeHeightedShape = Vec<NodeHeightedContour>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightSolution {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) regions: Vec<NodeHeightedRegion>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: usize,
    pub(crate) shape: NodeHeightedShape,
    pub(crate) area_m2: f32,
    pub(crate) height_sources: Vec<NodeHeightSource>,
    pub(crate) seam_constraints: Vec<NodeRegionSeamConstraint>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightedVertex {
    pub(crate) point_xz: RoadVec2,
    pub(crate) height_m: f64,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) height_sources: Vec<NodeHeightSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeHeightSourceError {
    InputOwnershipMismatch {
        input_node_id: u32,
        ownership_node_id: u32,
        input_piece_kind: RoadSurfaceVisualNodePieceKind,
        ownership_piece_kind: RoadSurfaceVisualNodePieceKind,
    },
    DuplicateSourceBand {
        mouth_order_index: usize,
        band_index: usize,
    },
    MissingRegionBandIndex {
        mouth_order_index: usize,
        kind: RoadSurfaceBandKind,
    },
    MissingSourceBand {
        mouth_order_index: usize,
        band_index: usize,
    },
    SourceBandKindMismatch {
        mouth_order_index: usize,
        band_index: usize,
        region_kind: RoadSurfaceBandKind,
        source_kind: RoadSurfaceBandKind,
    },
    DegenerateHeightField {
        mouth_order_index: usize,
        band_index: usize,
        axis: &'static str,
    },
    VertexOutsideHeightField {
        mouth_order_index: usize,
        band_index: usize,
        point_x_mm: i64,
        point_z_mm: i64,
        axis: &'static str,
        raw_parameter: f64,
    },
    HeightSampleFailed {
        mouth_order_index: usize,
        band_index: usize,
        axis: &'static str,
        parameter: f64,
    },
    SharedSourceHeightConflict {
        point_x_mm: i64,
        point_z_mm: i64,
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeHeightPointKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeSourceBandKey {
    mouth_order_index: usize,
    band_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeHeightVertexContextKey {
    point: NodeHeightPointKey,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

struct NodeBandHeightField {
    id: NodeBandHeightFieldId,
    kind: RoadSurfaceBandKind,
    endpoint_start_xz: RoadVec2,
    endpoint_end_xz: RoadVec2,
    mouth_start_xz: RoadVec2,
    mouth_end_xz: RoadVec2,
    start_height_profile: Spline<f64, f64>,
    end_height_profile: Spline<f64, f64>,
    height_sources: Vec<NodeHeightSource>,
}

struct NodeBoundaryHeightField {
    mouth_order_index: usize,
    boundary_index: usize,
    role: NodeInputBoundaryRailRole,
    endpoint_xz: RoadVec2,
    mouth_xz: RoadVec2,
    endpoint_height_m: f64,
    mouth_height_m: f64,
    height_sources: Vec<NodeHeightSource>,
}

impl RoadSurfaceSystem {
    pub(super) fn build_node_height_solution_from_ownership(
        input: &NodeArrangementInput,
        ownership: &NodeBooleanOwnership,
    ) -> Result<NodeHeightSolution, NodeHeightSourceError> {
        NodeHeightSolution::from_ownership_and_input(input, ownership)
    }
}

impl NodeHeightSolution {
    pub(crate) fn from_ownership_and_input(
        input: &NodeArrangementInput,
        ownership: &NodeBooleanOwnership,
    ) -> Result<Self, NodeHeightSourceError> {
        validate_input_ownership_pair(input, ownership)?;
        let fields = height_fields_by_source(input)?;
        let boundary_fields = boundary_height_fields(input);
        let mut regions = Vec::with_capacity(ownership.owned_regions.len());
        let global_points = global_ownership_points(ownership);

        for region in &ownership.owned_regions {
            let region = heighted_region(region, &fields, &boundary_fields, &global_points)?;
            if !region.shape.is_empty() {
                regions.push(region);
            }
        }
        validate_explicit_material_seam_heights(&regions)?;
        validate_shared_source_height_agreement(&regions)?;

        Ok(Self {
            node_id: ownership.node_id,
            piece_kind: ownership.piece_kind,
            regions,
        })
    }
}

impl NodeBandHeightField {
    fn from_interval(mouth_order_index: usize, interval: &NodeInputBandInterval) -> Self {
        let id =
            NodeBandHeightFieldId::new(mouth_order_index, interval.band_index, interval.band_kind);
        let endpoint_start_xz =
            quantize_road_vec2_to_overlay_grid(xz(interval.endpoint_start_world));
        let endpoint_end_xz = quantize_road_vec2_to_overlay_grid(xz(interval.endpoint_end_world));
        let mouth_start_xz = quantize_road_vec2_to_overlay_grid(xz(interval.mouth_start_world));
        let mouth_end_xz = quantize_road_vec2_to_overlay_grid(xz(interval.mouth_end_world));

        Self {
            id,
            kind: interval.band_kind,
            endpoint_start_xz,
            endpoint_end_xz,
            mouth_start_xz,
            mouth_end_xz,
            start_height_profile: linear_height_profile(
                interval.endpoint_start_world.y,
                interval.mouth_start_world.y,
            ),
            end_height_profile: linear_height_profile(
                interval.endpoint_end_world.y,
                interval.mouth_end_world.y,
            ),
            height_sources: canonical_height_sources([
                interval.endpoint_height_source.clone(),
                interval.mouth_height_source.clone(),
            ]),
        }
    }

    fn from_terminal_end_band(
        mouth_order_index: usize,
        end_band: &NodeInputTerminalEndBand,
    ) -> Self {
        let id = NodeBandHeightFieldId::new(
            mouth_order_index,
            end_band.source_band_index,
            end_band.band_kind,
        );
        let endpoint_start_xz = quantize_road_vec2_to_overlay_grid(xz(end_band.inner_start_world));
        let endpoint_end_xz = quantize_road_vec2_to_overlay_grid(xz(end_band.inner_end_world));
        let mouth_start_xz = quantize_road_vec2_to_overlay_grid(xz(end_band.outer_start_world));
        let mouth_end_xz = quantize_road_vec2_to_overlay_grid(xz(end_band.outer_end_world));

        Self {
            id,
            kind: end_band.band_kind,
            endpoint_start_xz,
            endpoint_end_xz,
            mouth_start_xz,
            mouth_end_xz,
            start_height_profile: linear_height_profile(
                end_band.inner_start_world.y,
                end_band.outer_start_world.y,
            ),
            end_height_profile: linear_height_profile(
                end_band.inner_end_world.y,
                end_band.outer_end_world.y,
            ),
            height_sources: canonical_height_sources(end_band.height_sources.iter().cloned()),
        }
    }

    fn evaluate_height(&self, point_xz: RoadVec2) -> Result<f64, NodeHeightSourceError> {
        let endpoint_center = midpoint(self.endpoint_start_xz, self.endpoint_end_xz);
        let mouth_center = midpoint(self.mouth_start_xz, self.mouth_end_xz);
        let longitudinal_axis = mouth_center - endpoint_center;
        let longitudinal_len2 = longitudinal_axis.length_squared();
        if longitudinal_len2 <= HEIGHT_FIELD_MIN_AXIS_LEN2_M2 {
            return Err(NodeHeightSourceError::DegenerateHeightField {
                mouth_order_index: self.id.mouth_order_index(),
                band_index: self.id.band_index(),
                axis: "longitudinal",
            });
        }
        let longitudinal_len_m = longitudinal_len2.sqrt();

        let raw_t = (point_xz - endpoint_center).dot(longitudinal_axis) / longitudinal_len2;
        let t = canonical_unit_parameter(raw_t, longitudinal_len_m)
            .ok_or_else(|| self.outside_field_error(point_xz, "longitudinal", raw_t))?;

        let start_xz = interpolate(self.endpoint_start_xz, self.mouth_start_xz, t);
        let end_xz = interpolate(self.endpoint_end_xz, self.mouth_end_xz, t);
        let lateral_axis = end_xz - start_xz;
        let lateral_len2 = lateral_axis.length_squared();
        if lateral_len2 <= HEIGHT_FIELD_MIN_AXIS_LEN2_M2 {
            return Err(NodeHeightSourceError::DegenerateHeightField {
                mouth_order_index: self.id.mouth_order_index(),
                band_index: self.id.band_index(),
                axis: "lateral",
            });
        }
        let lateral_len_m = lateral_len2.sqrt();

        let raw_u = (point_xz - start_xz).dot(lateral_axis) / lateral_len2;
        let u = canonical_unit_parameter(raw_u, lateral_len_m)
            .ok_or_else(|| self.outside_field_error(point_xz, "lateral", raw_u))?;
        let start_height = self.start_height_profile.clamped_sample(t).ok_or(
            NodeHeightSourceError::HeightSampleFailed {
                mouth_order_index: self.id.mouth_order_index(),
                band_index: self.id.band_index(),
                axis: "start",
                parameter: t,
            },
        )?;
        let end_height = self.end_height_profile.clamped_sample(t).ok_or(
            NodeHeightSourceError::HeightSampleFailed {
                mouth_order_index: self.id.mouth_order_index(),
                band_index: self.id.band_index(),
                axis: "end",
                parameter: t,
            },
        )?;

        Ok(start_height + (end_height - start_height) * u)
    }

    fn outside_field_error(
        &self,
        point_xz: RoadVec2,
        axis: &'static str,
        raw_parameter: f64,
    ) -> NodeHeightSourceError {
        let key = NodeHeightPointKey::from_point(point_xz);
        NodeHeightSourceError::VertexOutsideHeightField {
            mouth_order_index: self.id.mouth_order_index(),
            band_index: self.id.band_index(),
            point_x_mm: key.x_mm(),
            point_z_mm: key.z_mm(),
            axis,
            raw_parameter,
        }
    }
}

impl NodeBoundaryHeightField {
    fn from_boundary_rail(mouth: &NodeInputMouth, rail: &NodeInputBoundaryRail) -> Self {
        Self {
            mouth_order_index: mouth.order_index,
            boundary_index: rail.boundary_index,
            role: rail.role,
            endpoint_xz: quantize_road_vec2_to_overlay_grid(xz(rail.endpoint_world)),
            mouth_xz: quantize_road_vec2_to_overlay_grid(xz(rail.mouth_world)),
            endpoint_height_m: rail.endpoint_world.y,
            mouth_height_m: rail.mouth_world.y,
            height_sources: canonical_height_sources(
                mouth
                    .boundary_heights
                    .iter()
                    .filter(|height| height.boundary_index == rail.boundary_index)
                    .flat_map(|height| height.height_sources.iter().cloned()),
            ),
        }
    }

    fn from_terminal_end_band_edges(
        mouth_order_index: usize,
        end_band: &NodeInputTerminalEndBand,
    ) -> [Self; 2] {
        let inner_role = terminal_inner_boundary_role(end_band.band_kind);
        let outer_role = terminal_outer_boundary_role(end_band.band_kind);
        [
            Self {
                mouth_order_index,
                boundary_index: end_band.source_band_index * 2,
                role: inner_role,
                endpoint_xz: quantize_road_vec2_to_overlay_grid(xz(end_band.inner_start_world)),
                mouth_xz: quantize_road_vec2_to_overlay_grid(xz(end_band.inner_end_world)),
                endpoint_height_m: end_band.inner_start_world.y,
                mouth_height_m: end_band.inner_end_world.y,
                height_sources: canonical_height_sources(end_band.height_sources.iter().cloned()),
            },
            Self {
                mouth_order_index,
                boundary_index: end_band.source_band_index * 2 + 1,
                role: outer_role,
                endpoint_xz: quantize_road_vec2_to_overlay_grid(xz(end_band.outer_start_world)),
                mouth_xz: quantize_road_vec2_to_overlay_grid(xz(end_band.outer_end_world)),
                endpoint_height_m: end_band.outer_start_world.y,
                mouth_height_m: end_band.outer_end_world.y,
                height_sources: canonical_height_sources(end_band.height_sources.iter().cloned()),
            },
        ]
    }

    fn applies_to_kind(&self, kind: RoadSurfaceBandKind) -> bool {
        match self.role {
            NodeInputBoundaryRailRole::OuterFootprint { adjacent_kind } => adjacent_kind == kind,
            NodeInputBoundaryRailRole::InteriorBandBoundary {
                left_kind,
                right_kind,
            } => left_kind == kind || right_kind == kind,
        }
    }

    fn role_priority_for_kind(&self, kind: RoadSurfaceBandKind) -> usize {
        match (kind, self.role) {
            (
                RoadSurfaceBandKind::Carriageway,
                NodeInputBoundaryRailRole::InteriorBandBoundary {
                    left_kind,
                    right_kind,
                },
            ) if is_carriageway(left_kind) || is_carriageway(right_kind) => 0,
            (
                RoadSurfaceBandKind::CurbOrShoulder,
                NodeInputBoundaryRailRole::InteriorBandBoundary {
                    left_kind,
                    right_kind,
                },
            ) if is_carriageway(left_kind) || is_carriageway(right_kind) => 0,
            (
                RoadSurfaceBandKind::CurbOrShoulder,
                NodeInputBoundaryRailRole::InteriorBandBoundary {
                    left_kind,
                    right_kind,
                },
            ) if is_sidewalk(left_kind) || is_sidewalk(right_kind) => 1,
            (
                RoadSurfaceBandKind::Sidewalk,
                NodeInputBoundaryRailRole::InteriorBandBoundary {
                    left_kind,
                    right_kind,
                },
            ) if is_sidewalk(left_kind) || is_sidewalk(right_kind) => 0,
            (RoadSurfaceBandKind::Sidewalk, NodeInputBoundaryRailRole::OuterFootprint { .. }) => 1,
            (_, NodeInputBoundaryRailRole::OuterFootprint { .. }) => 2,
            _ => 3,
        }
    }

    fn sample_height(&self, point_xz: RoadVec2) -> Option<f64> {
        if !point_lies_on_road_segment(point_xz, self.endpoint_xz, self.mouth_xz) {
            return None;
        }
        let axis = self.mouth_xz - self.endpoint_xz;
        let len2 = axis.length_squared();
        if len2 <= HEIGHT_FIELD_MIN_AXIS_LEN2_M2 {
            return None;
        }
        let t = ((point_xz - self.endpoint_xz).dot(axis) / len2).clamp(0.0, 1.0);
        Some(self.endpoint_height_m + (self.mouth_height_m - self.endpoint_height_m) * t)
    }
}

fn validate_input_ownership_pair(
    input: &NodeArrangementInput,
    ownership: &NodeBooleanOwnership,
) -> Result<(), NodeHeightSourceError> {
    if input.node_id == ownership.node_id && input.piece_kind == ownership.piece_kind {
        return Ok(());
    }

    Err(NodeHeightSourceError::InputOwnershipMismatch {
        input_node_id: input.node_id,
        ownership_node_id: ownership.node_id,
        input_piece_kind: input.piece_kind,
        ownership_piece_kind: ownership.piece_kind,
    })
}

fn height_fields_by_source(
    input: &NodeArrangementInput,
) -> Result<BTreeMap<NodeSourceBandKey, NodeBandHeightField>, NodeHeightSourceError> {
    let mut fields = BTreeMap::new();
    for mouth in &input.mouths {
        for interval in &mouth.band_intervals {
            let field = NodeBandHeightField::from_interval(mouth.order_index, interval);
            let key = NodeSourceBandKey {
                mouth_order_index: mouth.order_index,
                band_index: interval.band_index,
            };
            if fields.insert(key, field).is_some() {
                return Err(NodeHeightSourceError::DuplicateSourceBand {
                    mouth_order_index: mouth.order_index,
                    band_index: interval.band_index,
                });
            }
        }
        for end_band in &mouth.terminal_end_bands {
            let field = NodeBandHeightField::from_terminal_end_band(mouth.order_index, end_band);
            let key = NodeSourceBandKey {
                mouth_order_index: mouth.order_index,
                band_index: end_band.source_band_index,
            };
            if fields.insert(key, field).is_some() {
                return Err(NodeHeightSourceError::DuplicateSourceBand {
                    mouth_order_index: mouth.order_index,
                    band_index: end_band.source_band_index,
                });
            }
        }
    }
    Ok(fields)
}

fn boundary_height_fields(input: &NodeArrangementInput) -> Vec<NodeBoundaryHeightField> {
    let mut fields = input
        .mouths
        .iter()
        .flat_map(|mouth| {
            let boundary_fields = mouth
                .boundary_rails
                .iter()
                .map(move |rail| NodeBoundaryHeightField::from_boundary_rail(mouth, rail));
            let terminal_fields = mouth.terminal_end_bands.iter().flat_map(move |end_band| {
                NodeBoundaryHeightField::from_terminal_end_band_edges(mouth.order_index, end_band)
            });
            boundary_fields.chain(terminal_fields)
        })
        .collect::<Vec<_>>();
    fields.sort_by(|a, b| {
        a.mouth_order_index
            .cmp(&b.mouth_order_index)
            .then(a.boundary_index.cmp(&b.boundary_index))
    });
    fields
}

fn terminal_inner_boundary_role(kind: RoadSurfaceBandKind) -> NodeInputBoundaryRailRole {
    match kind {
        RoadSurfaceBandKind::Carriageway => NodeInputBoundaryRailRole::InteriorBandBoundary {
            left_kind: RoadSurfaceBandKind::Carriageway,
            right_kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        RoadSurfaceBandKind::CurbOrShoulder => NodeInputBoundaryRailRole::InteriorBandBoundary {
            left_kind: RoadSurfaceBandKind::Carriageway,
            right_kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        RoadSurfaceBandKind::Sidewalk => NodeInputBoundaryRailRole::InteriorBandBoundary {
            left_kind: RoadSurfaceBandKind::CurbOrShoulder,
            right_kind: RoadSurfaceBandKind::Sidewalk,
        },
        _ => NodeInputBoundaryRailRole::OuterFootprint {
            adjacent_kind: kind,
        },
    }
}

fn terminal_outer_boundary_role(kind: RoadSurfaceBandKind) -> NodeInputBoundaryRailRole {
    match kind {
        RoadSurfaceBandKind::Carriageway => NodeInputBoundaryRailRole::InteriorBandBoundary {
            left_kind: RoadSurfaceBandKind::Carriageway,
            right_kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        RoadSurfaceBandKind::CurbOrShoulder => NodeInputBoundaryRailRole::InteriorBandBoundary {
            left_kind: RoadSurfaceBandKind::CurbOrShoulder,
            right_kind: RoadSurfaceBandKind::Sidewalk,
        },
        _ => NodeInputBoundaryRailRole::OuterFootprint {
            adjacent_kind: kind,
        },
    }
}

fn heighted_region(
    region: &NodeBooleanOwnedRegion,
    fields: &BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
    boundary_fields: &[NodeBoundaryHeightField],
    global_points: &[NodeOverlayPointKey],
) -> Result<NodeHeightedRegion, NodeHeightSourceError> {
    let band_index =
        region
            .source_band_index
            .ok_or(NodeHeightSourceError::MissingRegionBandIndex {
                mouth_order_index: region.source_mouth_order_index,
                kind: region.kind,
            })?;
    let key = NodeSourceBandKey {
        mouth_order_index: region.source_mouth_order_index,
        band_index,
    };
    let field = fields
        .get(&key)
        .ok_or(NodeHeightSourceError::MissingSourceBand {
            mouth_order_index: key.mouth_order_index,
            band_index: key.band_index,
        })?;
    if field.kind != region.kind {
        return Err(NodeHeightSourceError::SourceBandKindMismatch {
            mouth_order_index: key.mouth_order_index,
            band_index: key.band_index,
            region_kind: region.kind,
            source_kind: field.kind,
        });
    }

    let height_sources = canonical_height_sources(
        region
            .height_sources
            .iter()
            .cloned()
            .chain(field.height_sources.iter().cloned()),
    );
    let shape = heighted_shape(
        &region.shape,
        region.kind,
        field,
        boundary_fields,
        &height_sources,
        global_points,
    )?;

    Ok(NodeHeightedRegion {
        kind: region.kind,
        owner: region.owner,
        height_field_id: field.id,
        source_mouth_order_index: region.source_mouth_order_index,
        source_band_index: band_index,
        shape,
        area_m2: region.area_m2,
        height_sources,
        seam_constraints: region.seam_constraints.clone(),
    })
}

fn heighted_shape(
    shape: &NodeOverlayShape,
    kind: RoadSurfaceBandKind,
    field: &NodeBandHeightField,
    boundary_fields: &[NodeBoundaryHeightField],
    height_sources: &[NodeHeightSource],
    global_points: &[NodeOverlayPointKey],
) -> Result<NodeHeightedShape, NodeHeightSourceError> {
    let mut heighted = Vec::with_capacity(shape.len());
    for contour in shape {
        let contour = heighted_contour(
            contour,
            kind,
            field,
            boundary_fields,
            height_sources,
            global_points,
        )?;
        if contour.len() >= 3 {
            heighted.push(contour);
        }
    }
    Ok(heighted)
}

fn heighted_contour(
    contour: &NodeOverlayContour,
    kind: RoadSurfaceBandKind,
    field: &NodeBandHeightField,
    boundary_fields: &[NodeBoundaryHeightField],
    height_sources: &[NodeHeightSource],
    global_points: &[NodeOverlayPointKey],
) -> Result<NodeHeightedContour, NodeHeightSourceError> {
    let contour = noded_overlay_contour(contour, global_points);
    contour
        .into_iter()
        .map(|point| heighted_vertex(point, kind, field, boundary_fields, height_sources))
        .collect()
}

fn heighted_vertex(
    point: NodeOverlayPoint,
    kind: RoadSurfaceBandKind,
    field: &NodeBandHeightField,
    boundary_fields: &[NodeBoundaryHeightField],
    height_sources: &[NodeHeightSource],
) -> Result<NodeHeightedVertex, NodeHeightSourceError> {
    let point_xz = overlay_point_to_road(point);
    let mut vertex = if let Some((height_m, sources)) = boundary_height_at_point(
        kind,
        field.id.mouth_order_index(),
        point_xz,
        boundary_fields,
    ) {
        NodeHeightedVertex {
            point_xz,
            height_m,
            height_field_id: field.id,
            height_sources: canonical_height_sources(
                height_sources
                    .iter()
                    .cloned()
                    .chain(sources.iter().cloned()),
            ),
        }
    } else {
        NodeHeightedVertex {
            point_xz,
            height_m: field.evaluate_height(point_xz)?,
            height_field_id: field.id,
            height_sources: height_sources.to_vec(),
        }
    };
    vertex.height_sources = canonical_height_sources(vertex.height_sources);
    Ok(vertex)
}

fn boundary_height_at_point(
    kind: RoadSurfaceBandKind,
    source_mouth_order_index: usize,
    point_xz: RoadVec2,
    fields: &[NodeBoundaryHeightField],
) -> Option<(f64, Vec<NodeHeightSource>)> {
    fields
        .iter()
        .filter(|field| field.mouth_order_index == source_mouth_order_index)
        .filter(|field| field.applies_to_kind(kind))
        .filter_map(|field| {
            field.sample_height(point_xz).map(|height_m| {
                (
                    field.role_priority_for_kind(kind),
                    field.mouth_order_index,
                    field.boundary_index,
                    height_m,
                    field.height_sources.clone(),
                )
            })
        })
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)))
        .map(|(_, _, _, height_m, sources)| (height_m, sources))
}

fn is_carriageway(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Carriageway
}

fn is_sidewalk(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Sidewalk
}

fn point_lies_on_road_segment(point: RoadVec2, start: RoadVec2, end: RoadVec2) -> bool {
    let point = road_overlay_point_key(point);
    let start = road_overlay_point_key(start);
    let end = road_overlay_point_key(end);
    if point == start || point == end {
        return true;
    }
    point_lies_strictly_inside_segment(point, start, end)
}

fn road_overlay_point_key(point: RoadVec2) -> NodeOverlayPointKey {
    (
        (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn validate_explicit_material_seam_heights(
    regions: &[NodeHeightedRegion],
) -> Result<(), NodeHeightSourceError> {
    let mut shared_heights = BTreeMap::<(NodeHeightPointKey, usize), ExplicitSeamHeight>::new();
    for region in regions.iter() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            for constraint in
                material_height_constraints_for_vertex(vertex.point_xz, &region.seam_constraints)
            {
                let point = NodeHeightPointKey::from_point(vertex.point_xz);
                let key = (point, constraint.constraint_index);
                let height_mm = quantize_m(vertex.height_m);
                if let Some(existing) = shared_heights.insert(key, ExplicitSeamHeight { height_mm })
                {
                    if existing.height_mm != height_mm {
                        return Err(NodeHeightSourceError::SharedSourceHeightConflict {
                            point_x_mm: point.x_mm(),
                            point_z_mm: point.z_mm(),
                            kind: region.kind,
                            owner_index: region.owner.owner_index(),
                            existing_height_mm: existing.height_mm,
                            incoming_height_mm: height_mm,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct ExplicitSeamHeight {
    height_mm: i64,
}

fn material_height_constraints_for_vertex<'a>(
    point_xz: RoadVec2,
    constraints: &'a [NodeRegionSeamConstraint],
) -> Vec<&'a NodeRegionSeamConstraint> {
    let mut matches = constraints
        .iter()
        .filter(|constraint| constraint.constrains_shared_height)
        .filter(|constraint| {
            point_lies_on_height_segment(point_xz, constraint.start_xz, constraint.end_xz)
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|constraint| {
        (
            !constraint.is_material_transition,
            seam_source_priority(&constraint.seam_source),
            constraint.constraint_index,
        )
    });
    matches.dedup_by_key(|constraint| constraint.constraint_index);
    matches
}

fn point_lies_on_height_segment(point: RoadVec2, start: RoadVec2, end: RoadVec2) -> bool {
    let point = NodeHeightPointKey::from_point(point);
    let start = NodeHeightPointKey::from_point(start);
    let end = NodeHeightPointKey::from_point(end);
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.x_key - start.x_key);
    let dz = i128::from(end.z_key - start.z_key);
    let px = i128::from(point.x_key - start.x_key);
    let pz = i128::from(point.z_key - start.z_key);
    if px * dz - pz * dx != 0 {
        return false;
    }
    let inside_x = if start.x_key == end.x_key {
        point.x_key == start.x_key
    } else {
        point.x_key > start.x_key.min(end.x_key) && point.x_key < start.x_key.max(end.x_key)
    };
    let inside_z = if start.z_key == end.z_key {
        point.z_key == start.z_key
    } else {
        point.z_key > start.z_key.min(end.z_key) && point.z_key < start.z_key.max(end.z_key)
    };
    inside_x && inside_z
}

fn validate_shared_source_height_agreement(
    regions: &[NodeHeightedRegion],
) -> Result<(), NodeHeightSourceError> {
    let mut heights = BTreeMap::<NodeHeightVertexContextKey, i64>::new();
    for region in regions {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            let point = NodeHeightPointKey::from_point(vertex.point_xz);
            let key = NodeHeightVertexContextKey {
                point,
                owner: region.owner,
                height_field_id: vertex.height_field_id,
            };
            let height_mm = quantize_m(vertex.height_m);
            if let Some(existing_height_mm) = heights.insert(key, height_mm)
                && existing_height_mm != height_mm
            {
                return Err(NodeHeightSourceError::SharedSourceHeightConflict {
                    point_x_mm: point.x_mm(),
                    point_z_mm: point.z_mm(),
                    kind: region.kind,
                    owner_index: region.owner.owner_index(),
                    existing_height_mm,
                    incoming_height_mm: height_mm,
                });
            }
        }
    }
    Ok(())
}

fn linear_height_profile(endpoint_height_m: f64, mouth_height_m: f64) -> Spline<f64, f64> {
    Spline::from_vec(vec![
        Key::new(0.0, endpoint_height_m, Interpolation::Linear),
        Key::new(1.0, mouth_height_m, Interpolation::Linear),
    ])
}

fn canonical_unit_parameter(raw_parameter: f64, axis_length_m: f64) -> Option<f64> {
    if !raw_parameter.is_finite() {
        return None;
    }

    let parameter =
        (raw_parameter * HEIGHT_PARAMETER_KEY_SCALE).round() / HEIGHT_PARAMETER_KEY_SCALE;
    let boundary_eps = if axis_length_m > f64::EPSILON {
        HEIGHT_PARAMETER_BOUNDARY_EPS.max(HEIGHT_PARAMETER_BOUNDARY_DISTANCE_EPS_M / axis_length_m)
    } else {
        HEIGHT_PARAMETER_BOUNDARY_EPS
    };
    (-boundary_eps..=1.0 + boundary_eps)
        .contains(&parameter)
        .then_some(parameter.clamp(0.0, 1.0))
}

fn canonical_height_sources(
    sources: impl IntoIterator<Item = NodeHeightSource>,
) -> Vec<NodeHeightSource> {
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources
}

fn xz(point: RoadVec3) -> RoadVec2 {
    RoadVec2::new(point.x, point.z)
}

fn midpoint(start: RoadVec2, end: RoadVec2) -> RoadVec2 {
    (start + end) * 0.5
}

fn interpolate(start: RoadVec2, end: RoadVec2, t: f64) -> RoadVec2 {
    start + (end - start) * t
}

type NodeOverlayPointKey = (i64, i64);

fn global_ownership_points(ownership: &NodeBooleanOwnership) -> Vec<NodeOverlayPointKey> {
    let mut points = ownership
        .owned_regions
        .iter()
        .flat_map(|region| region.shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(overlay_point_key)
        .collect::<Vec<_>>();
    points.extend(
        ownership
            .footprint_shapes
            .iter()
            .flat_map(|shape| shape.iter())
            .flat_map(|contour| contour.iter().copied())
            .map(overlay_point_key),
    );
    points.sort_unstable();
    points.dedup();
    points
}

fn noded_overlay_contour(
    contour: &NodeOverlayContour,
    global_points: &[NodeOverlayPointKey],
) -> NodeOverlayContour {
    if contour.len() < 2 {
        return contour.clone();
    }

    let mut noded = Vec::with_capacity(contour.len());
    for edge_index in 0..contour.len() {
        let start = overlay_point_key(contour[edge_index]);
        let end = overlay_point_key(contour[(edge_index + 1) % contour.len()]);
        noded.push(overlay_point_from_key(start));
        let mut split_points = global_points
            .iter()
            .copied()
            .filter(|point| point_lies_strictly_inside_segment(*point, start, end))
            .filter(|point| !point_is_numeric_endpoint_split(*point, start, end))
            .collect::<Vec<_>>();
        sort_segment_split_points(start, end, &mut split_points);
        noded.extend(split_points.into_iter().map(overlay_point_from_key));
    }

    dedup_consecutive_overlay_points(&mut noded);
    if noded.len() >= 2
        && overlay_point_key(*noded.first().expect("noded contour has first point"))
            == overlay_point_key(*noded.last().expect("noded contour has last point"))
    {
        noded.pop();
    }
    remove_overlay_spikes(&mut noded);
    noded
}

fn point_is_numeric_endpoint_split(
    point: NodeOverlayPointKey,
    start: NodeOverlayPointKey,
    end: NodeOverlayPointKey,
) -> bool {
    let key_epsilon =
        (HEIGHT_PARAMETER_BOUNDARY_EPS * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64;
    point_key_linf_distance(point, start) <= key_epsilon
        || point_key_linf_distance(point, end) <= key_epsilon
}

fn point_key_linf_distance(a: NodeOverlayPointKey, b: NodeOverlayPointKey) -> i64 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

fn remove_overlay_spikes(points: &mut NodeOverlayContour) {
    if points.len() < 3 {
        return;
    }

    let mut changed = true;
    while changed && points.len() >= 3 {
        changed = false;
        let len = points.len();
        for index in 0..len {
            let prev = if index == 0 { len - 1 } else { index - 1 };
            let next = if index + 1 == len { 0 } else { index + 1 };
            if height_overlay_point_key(points[prev]) == height_overlay_point_key(points[next]) {
                points.remove(index);
                dedup_consecutive_overlay_points(points);
                changed = true;
                break;
            }
        }
    }
}

fn point_lies_strictly_inside_segment(
    point: NodeOverlayPointKey,
    start: NodeOverlayPointKey,
    end: NodeOverlayPointKey,
) -> bool {
    if point == start || point == end || start == end {
        return false;
    }
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    if px * dz - pz * dx != 0 {
        return false;
    }
    let inside_x = if start.0 == end.0 {
        point.0 == start.0
    } else {
        point.0 > start.0.min(end.0) && point.0 < start.0.max(end.0)
    };
    let inside_z = if start.1 == end.1 {
        point.1 == start.1
    } else {
        point.1 > start.1.min(end.1) && point.1 < start.1.max(end.1)
    };
    inside_x && inside_z
}

fn sort_segment_split_points(
    start: NodeOverlayPointKey,
    end: NodeOverlayPointKey,
    points: &mut Vec<NodeOverlayPointKey>,
) {
    let dx = end.0 - start.0;
    let dz = end.1 - start.1;
    if dx.abs() >= dz.abs() {
        points.sort_by_key(|point| {
            if dx >= 0 {
                point.0 - start.0
            } else {
                start.0 - point.0
            }
        });
    } else {
        points.sort_by_key(|point| {
            if dz >= 0 {
                point.1 - start.1
            } else {
                start.1 - point.1
            }
        });
    }
    points.dedup();
}

fn dedup_consecutive_overlay_points(points: &mut NodeOverlayContour) {
    points.dedup_by(|a, b| overlay_point_key(*a) == overlay_point_key(*b));
}

fn overlay_point_key(point: NodeOverlayPoint) -> NodeOverlayPointKey {
    (
        (point[0] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point[1] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn height_overlay_point_key(point: NodeOverlayPoint) -> NodeOverlayPointKey {
    (
        (point[0] * HEIGHT_KEY_SCALE).round() as i64,
        (point[1] * HEIGHT_KEY_SCALE).round() as i64,
    )
}

fn overlay_point_from_key(point: NodeOverlayPointKey) -> NodeOverlayPoint {
    [
        point.0 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
        point.1 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
    ]
}

fn quantize_m(value: f64) -> i64 {
    (value * HEIGHT_KEY_SCALE).round() as i64
}

impl NodeHeightPointKey {
    fn from_point(point: RoadVec2) -> Self {
        Self {
            x_key: (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
            z_key: (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        }
    }

    fn x_mm(self) -> i64 {
        ((self.x_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE) * HEIGHT_KEY_SCALE).round() as i64
    }

    fn z_mm(self) -> i64 {
        ((self.z_key as f64 / ROAD_OVERLAY_COORDINATE_SCALE) * HEIGHT_KEY_SCALE).round() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::super::arrangement::NodeSeamSource;
    use super::*;
    use crate::simulation::network::surface::input::NodeInputMouth;
    use crate::simulation::network::surface::ownership::{
        NodeBooleanOwnership, NodeOwnedRegionArrangement,
    };
    use crate::simulation::network::surface::rails::NodeRailContourSet;
    use crate::simulation::network::surface::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use godot::prelude::{Vector2, Vector3};

    fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn profile(x: f32, base_height: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, base_height, -4.0),
            Vector3::new(x, base_height + 0.1, -2.0),
            Vector3::new(x, base_height + 0.2, 0.0),
            Vector3::new(x, base_height + 0.3, 2.0),
            Vector3::new(x, base_height + 0.4, 4.0),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: Vector2::RIGHT,
            boundary_points_world,
            bands,
        }
    }

    fn solved_input() -> NodeArrangementInput {
        let mouth = OrderedIncidentPieceMouth {
            profile: profile(10.0, 4.0),
            endpoint_profile: profile(0.0, 2.0),
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[mouth],
        )
        .expect("test mouth should produce canonical input")
    }

    fn solved_ownership(input: &NodeArrangementInput) -> NodeBooleanOwnership {
        let rails = NodeRailContourSet::from_input(input).expect("test input should produce rails");
        NodeBooleanOwnership::from_rails(&rails).expect("test rails should produce ownership")
    }

    #[test]
    fn evaluates_owned_region_vertices_from_band_height_fields() {
        let input = solved_input();
        let ownership = solved_ownership(&input);
        let solution = NodeHeightSolution::from_ownership_and_input(&input, &ownership)
            .expect("valid ownership should height every canonical vertex");

        assert_eq!(solution.node_id, 42);
        assert_eq!(
            solution.piece_kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_eq!(solution.regions.len(), ownership.owned_regions.len());

        let carriageway = solution
            .regions
            .iter()
            .find(|region| region.kind == RoadSurfaceBandKind::Carriageway)
            .expect("test input has a carriageway band");
        assert!(has_vertex_height(carriageway, 0.0, 0.0, 2.2));
        assert!(has_vertex_height(carriageway, 10.0, 2.0, 4.3));
        assert!(!carriageway.height_sources.is_empty());
    }

    #[test]
    fn rejects_missing_source_band() {
        let input = conflicting_manual_input();
        let owned_regions = vec![manual_region(RoadSurfaceBandKind::Carriageway, 99, 2.0)];
        let ownership = NodeBooleanOwnership {
            node_id: 77,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            footprint_shapes: Vec::new(),
            asphalt_shapes: Vec::new(),
            non_road_shapes: Vec::new(),
            owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
                77,
                RoadSurfaceVisualNodePieceKind::Bend,
                &owned_regions,
                &Vec::new(),
            ),
            owned_regions,
        };

        assert_eq!(
            NodeHeightSolution::from_ownership_and_input(&input, &ownership),
            Err(NodeHeightSourceError::MissingSourceBand {
                mouth_order_index: 0,
                band_index: 99,
            })
        );
    }

    #[test]
    fn shared_xz_vertices_keep_distinct_owner_source_heights() {
        let regions = vec![
            manual_heighted_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            ),
            manual_heighted_region(
                RoadSurfaceBandKind::Sidewalk,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            ),
        ];

        validate_shared_source_height_agreement(&regions)
            .expect("different owner/source contexts are explicit seams, not height repairs");

        assert_eq!(regions[0].shape[0][0].height_m, 0.0);
        assert_eq!(regions[1].shape[0][0].height_m, 0.25);
    }

    #[test]
    fn shared_xz_vertices_without_explicit_seam_are_not_height_constrained() {
        let regions = vec![
            manual_heighted_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            ),
            manual_heighted_region(
                RoadSurfaceBandKind::Sidewalk,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            ),
        ];

        validate_explicit_material_seam_heights(&regions)
            .expect("missing explicit seam must not trigger coincident-XZ height repair");

        assert_eq!(regions[0].shape[0][0].height_m, 0.0);
        assert_eq!(regions[1].shape[0][0].height_m, 0.25);
    }

    #[test]
    fn explicit_curb_sidewalk_seam_rejects_shared_height_disagreement() {
        let seam = manual_seam_constraint(
            12,
            NodeSeamSource::CurbSidewalkContact { owner_index: 0 },
            true,
            true,
        );
        let regions = vec![
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
                vec![seam.clone()],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
                vec![seam],
            ),
        ];

        assert!(matches!(
            validate_explicit_material_seam_heights(&regions),
            Err(NodeHeightSourceError::SharedSourceHeightConflict { .. })
        ));
    }

    #[test]
    fn explicit_curb_sidewalk_seam_accepts_matching_shared_height() {
        let seam = manual_seam_constraint(
            12,
            NodeSeamSource::CurbSidewalkContact { owner_index: 0 },
            true,
            true,
        );
        let regions = vec![
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
                vec![seam.clone()],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
                vec![seam],
            ),
        ];

        validate_explicit_material_seam_heights(&regions)
            .expect("explicit seam authority may only accept already matching heights");
    }

    #[test]
    fn generic_contour_seams_reject_shared_height_disagreement() {
        let seam = manual_seam_constraint(
            3,
            NodeSeamSource::AsphaltCurbContact { owner_index: 0 },
            true,
            false,
        );
        let regions = vec![
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
                vec![seam.clone()],
            ),
            manual_heighted_region_with_seams(
                RoadSurfaceBandKind::CurbOrShoulder,
                1,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
                vec![seam],
            ),
        ];

        assert!(matches!(
            validate_explicit_material_seam_heights(&regions),
            Err(NodeHeightSourceError::SharedSourceHeightConflict { .. })
        ));
    }

    #[test]
    fn shared_xz_vertices_reject_same_source_height_conflict() {
        let regions = vec![
            manual_heighted_region(
                RoadSurfaceBandKind::Sidewalk,
                0,
                0.0,
                vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            ),
            manual_heighted_region(
                RoadSurfaceBandKind::Sidewalk,
                0,
                0.25,
                vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            ),
        ];

        assert!(matches!(
            validate_shared_source_height_agreement(&regions),
            Err(NodeHeightSourceError::SharedSourceHeightConflict { .. })
        ));
    }

    fn has_vertex_height(
        region: &NodeHeightedRegion,
        expected_x: f64,
        expected_z: f64,
        expected_height: f64,
    ) -> bool {
        region.shape.iter().flatten().any(|vertex| {
            (vertex.point_xz.x - expected_x).abs() <= 1.0e-6
                && (vertex.point_xz.y - expected_z).abs() <= 1.0e-6
                && (vertex.height_m - expected_height).abs() <= 1.0e-6
        })
    }

    fn conflicting_manual_input() -> NodeArrangementInput {
        NodeArrangementInput {
            node_id: 77,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            mouths: vec![NodeInputMouth {
                order_index: 0,
                edge_idx: 9,
                side: IncidentEdgeSide::Start,
                direction_xz: RoadVec2::X,
                direction_angle_ccw: 0.0,
                conflict_handoff_distance_m: 10.0,
                mouth_rails: Vec::new(),
                endpoint_rails: Vec::new(),
                boundary_rails: Vec::new(),
                band_intervals: vec![
                    manual_interval(0, RoadSurfaceBandKind::Carriageway, 2.0, 4.0),
                    manual_interval(1, RoadSurfaceBandKind::Sidewalk, 5.0, 7.0),
                ],
                terminal_end_bands: Vec::new(),
                boundary_heights: Vec::new(),
            }],
        }
    }

    fn manual_interval(
        band_index: usize,
        band_kind: RoadSurfaceBandKind,
        endpoint_height: f64,
        mouth_height: f64,
    ) -> NodeInputBandInterval {
        NodeInputBandInterval {
            band_index,
            band_kind,
            mouth_start_world: RoadVec3::new(10.0, mouth_height, 0.0),
            mouth_end_world: RoadVec3::new(10.0, mouth_height, 2.0),
            endpoint_start_world: RoadVec3::new(0.0, endpoint_height, 0.0),
            endpoint_end_world: RoadVec3::new(0.0, endpoint_height, 2.0),
            mouth_height_source: NodeHeightSource::IncidentMouthBand {
                edge_idx: 9,
                side: IncidentEdgeSide::Start,
                band_index,
            },
            endpoint_height_source: NodeHeightSource::EndpointBand {
                edge_idx: 9,
                side: IncidentEdgeSide::Start,
                band_index,
            },
        }
    }

    fn manual_region(
        kind: RoadSurfaceBandKind,
        band_index: usize,
        area_m2: f32,
    ) -> NodeBooleanOwnedRegion {
        NodeBooleanOwnedRegion {
            kind,
            owner: NodeBandOwner::new(kind, band_index),
            source_mouth_order_index: 0,
            source_band_index: Some(band_index),
            shape: vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 2.0], [0.0, 2.0]]],
            area_m2,
            height_sources: Vec::new(),
            seam_constraints: Vec::new(),
        }
    }

    fn manual_heighted_region(
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        area_m2: f32,
        contour: NodeHeightedContour,
    ) -> NodeHeightedRegion {
        manual_heighted_region_with_seams(kind, owner_index, area_m2, contour, Vec::new())
    }

    fn manual_heighted_region_with_seams(
        kind: RoadSurfaceBandKind,
        owner_index: usize,
        area_m2: f32,
        contour: NodeHeightedContour,
        seam_constraints: Vec<NodeRegionSeamConstraint>,
    ) -> NodeHeightedRegion {
        let height_field_id = NodeBandHeightFieldId::new(owner_index, owner_index, kind);
        let contour = contour
            .into_iter()
            .map(|mut vertex| {
                vertex.height_field_id = height_field_id;
                vertex
            })
            .collect();
        NodeHeightedRegion {
            kind,
            owner: NodeBandOwner::new(kind, owner_index),
            height_field_id,
            source_mouth_order_index: owner_index,
            source_band_index: owner_index,
            shape: vec![contour],
            area_m2,
            height_sources: vec![NodeHeightSource::ArrangementConstraint {
                constraint_index: owner_index,
            }],
            seam_constraints,
        }
    }

    fn manual_seam_constraint(
        constraint_index: usize,
        seam_source: NodeSeamSource,
        constrains_shared_height: bool,
        is_material_transition: bool,
    ) -> NodeRegionSeamConstraint {
        NodeRegionSeamConstraint {
            constraint_index,
            seam_source,
            constrains_shared_height,
            is_material_transition,
            start_xz: RoadVec2::new(0.0, 0.0),
            end_xz: RoadVec2::new(1.0, 0.0),
        }
    }

    fn manual_heighted_vertex(x: f64, z: f64, height_m: f64) -> NodeHeightedVertex {
        NodeHeightedVertex {
            point_xz: RoadVec2::new(x, z),
            height_m,
            height_field_id: NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk),
            height_sources: Vec::new(),
        }
    }
}
