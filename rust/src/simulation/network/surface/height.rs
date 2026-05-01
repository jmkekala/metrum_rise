//! Spline-backed height evaluation for canonical node-owned regions.

#![allow(dead_code)]

use super::arrangement::{NodeBandOwner, NodeHeightSource};
use super::backend::{RoadVec2, RoadVec3, overlay_point_to_road};
use super::input::{NodeArrangementInput, NodeInputBandInterval};
use super::ownership::{NodeBooleanOwnedRegion, NodeBooleanOwnership};
use super::{
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind,
};
use splines::{Interpolation, Key, Spline};
use std::collections::BTreeMap;

const HEIGHT_KEY_SCALE: f64 = 1000.0;
const HEIGHT_PARAMETER_KEY_SCALE: f64 = 1_000_000.0;
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
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: usize,
    pub(crate) shape: NodeHeightedShape,
    pub(crate) area_m2: f32,
    pub(crate) height_sources: Vec<NodeHeightSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightedVertex {
    pub(crate) point_xz: RoadVec2,
    pub(crate) height_m: f64,
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
    SameXzHeightConflict {
        x_mm: i64,
        z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeHeightPointKey {
    x_mm: i64,
    z_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeHeightValueKey(i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeSourceBandKey {
    mouth_order_index: usize,
    band_index: usize,
}

struct NodeBandHeightField {
    key: NodeSourceBandKey,
    kind: RoadSurfaceBandKind,
    endpoint_start_xz: RoadVec2,
    endpoint_end_xz: RoadVec2,
    mouth_start_xz: RoadVec2,
    mouth_end_xz: RoadVec2,
    start_height_profile: Spline<f64, f64>,
    end_height_profile: Spline<f64, f64>,
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
        let mut height_by_point = BTreeMap::new();
        let mut regions = Vec::with_capacity(ownership.owned_regions.len());

        for region in &ownership.owned_regions {
            regions.push(heighted_region(region, &fields, &mut height_by_point)?);
        }

        Ok(Self {
            node_id: ownership.node_id,
            piece_kind: ownership.piece_kind,
            regions,
        })
    }
}

impl NodeBandHeightField {
    fn from_interval(mouth_order_index: usize, interval: &NodeInputBandInterval) -> Self {
        let key = NodeSourceBandKey {
            mouth_order_index,
            band_index: interval.band_index,
        };
        let endpoint_start_xz = xz(interval.endpoint_start_world);
        let endpoint_end_xz = xz(interval.endpoint_end_world);
        let mouth_start_xz = xz(interval.mouth_start_world);
        let mouth_end_xz = xz(interval.mouth_end_world);

        Self {
            key,
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

    fn evaluate_height(&self, point_xz: RoadVec2) -> Result<f64, NodeHeightSourceError> {
        let endpoint_center = midpoint(self.endpoint_start_xz, self.endpoint_end_xz);
        let mouth_center = midpoint(self.mouth_start_xz, self.mouth_end_xz);
        let longitudinal_axis = mouth_center - endpoint_center;
        let longitudinal_len2 = longitudinal_axis.length_squared();
        if longitudinal_len2 <= HEIGHT_FIELD_MIN_AXIS_LEN2_M2 {
            return Err(NodeHeightSourceError::DegenerateHeightField {
                mouth_order_index: self.key.mouth_order_index,
                band_index: self.key.band_index,
                axis: "longitudinal",
            });
        }

        let raw_t = (point_xz - endpoint_center).dot(longitudinal_axis) / longitudinal_len2;
        let t = canonical_unit_parameter(raw_t)
            .ok_or_else(|| self.outside_field_error(point_xz, "longitudinal", raw_t))?;

        let start_xz = interpolate(self.endpoint_start_xz, self.mouth_start_xz, t);
        let end_xz = interpolate(self.endpoint_end_xz, self.mouth_end_xz, t);
        let lateral_axis = end_xz - start_xz;
        let lateral_len2 = lateral_axis.length_squared();
        if lateral_len2 <= HEIGHT_FIELD_MIN_AXIS_LEN2_M2 {
            return Err(NodeHeightSourceError::DegenerateHeightField {
                mouth_order_index: self.key.mouth_order_index,
                band_index: self.key.band_index,
                axis: "lateral",
            });
        }

        let raw_u = (point_xz - start_xz).dot(lateral_axis) / lateral_len2;
        let u = canonical_unit_parameter(raw_u)
            .ok_or_else(|| self.outside_field_error(point_xz, "lateral", raw_u))?;
        let start_height = self.start_height_profile.clamped_sample(t).ok_or(
            NodeHeightSourceError::HeightSampleFailed {
                mouth_order_index: self.key.mouth_order_index,
                band_index: self.key.band_index,
                axis: "start",
                parameter: t,
            },
        )?;
        let end_height = self.end_height_profile.clamped_sample(t).ok_or(
            NodeHeightSourceError::HeightSampleFailed {
                mouth_order_index: self.key.mouth_order_index,
                band_index: self.key.band_index,
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
            mouth_order_index: self.key.mouth_order_index,
            band_index: self.key.band_index,
            point_x_mm: key.x_mm,
            point_z_mm: key.z_mm,
            axis,
            raw_parameter,
        }
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
            if fields.insert(field.key, field).is_some() {
                return Err(NodeHeightSourceError::DuplicateSourceBand {
                    mouth_order_index: mouth.order_index,
                    band_index: interval.band_index,
                });
            }
        }
    }
    Ok(fields)
}

fn heighted_region(
    region: &NodeBooleanOwnedRegion,
    fields: &BTreeMap<NodeSourceBandKey, NodeBandHeightField>,
    height_by_point: &mut BTreeMap<NodeHeightPointKey, NodeHeightValueKey>,
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
    let shape = heighted_shape(&region.shape, field, &height_sources, height_by_point)?;

    Ok(NodeHeightedRegion {
        kind: region.kind,
        owner: region.owner,
        source_mouth_order_index: region.source_mouth_order_index,
        source_band_index: band_index,
        shape,
        area_m2: region.area_m2,
        height_sources,
    })
}

fn heighted_shape(
    shape: &NodeOverlayShape,
    field: &NodeBandHeightField,
    height_sources: &[NodeHeightSource],
    height_by_point: &mut BTreeMap<NodeHeightPointKey, NodeHeightValueKey>,
) -> Result<NodeHeightedShape, NodeHeightSourceError> {
    shape
        .iter()
        .map(|contour| heighted_contour(contour, field, height_sources, height_by_point))
        .collect()
}

fn heighted_contour(
    contour: &NodeOverlayContour,
    field: &NodeBandHeightField,
    height_sources: &[NodeHeightSource],
    height_by_point: &mut BTreeMap<NodeHeightPointKey, NodeHeightValueKey>,
) -> Result<NodeHeightedContour, NodeHeightSourceError> {
    contour
        .iter()
        .copied()
        .map(|point| heighted_vertex(point, field, height_sources, height_by_point))
        .collect()
}

fn heighted_vertex(
    point: NodeOverlayPoint,
    field: &NodeBandHeightField,
    height_sources: &[NodeHeightSource],
    height_by_point: &mut BTreeMap<NodeHeightPointKey, NodeHeightValueKey>,
) -> Result<NodeHeightedVertex, NodeHeightSourceError> {
    let point_xz = overlay_point_to_road(point);
    let height_m = field.evaluate_height(point_xz)?;
    reject_same_xz_height_conflict(point_xz, height_m, height_by_point)?;
    Ok(NodeHeightedVertex {
        point_xz,
        height_m,
        height_sources: height_sources.to_vec(),
    })
}

fn reject_same_xz_height_conflict(
    point_xz: RoadVec2,
    height_m: f64,
    height_by_point: &mut BTreeMap<NodeHeightPointKey, NodeHeightValueKey>,
) -> Result<(), NodeHeightSourceError> {
    let point_key = NodeHeightPointKey::from_point(point_xz);
    let height_key = NodeHeightValueKey::from_height(height_m);
    if let Some(existing_height_key) = height_by_point.get(&point_key).copied() {
        if existing_height_key != height_key {
            return Err(NodeHeightSourceError::SameXzHeightConflict {
                x_mm: point_key.x_mm,
                z_mm: point_key.z_mm,
                existing_height_mm: existing_height_key.0,
                incoming_height_mm: height_key.0,
            });
        }
        return Ok(());
    }

    height_by_point.insert(point_key, height_key);
    Ok(())
}

fn linear_height_profile(endpoint_height_m: f64, mouth_height_m: f64) -> Spline<f64, f64> {
    Spline::from_vec(vec![
        Key::new(0.0, endpoint_height_m, Interpolation::Linear),
        Key::new(1.0, mouth_height_m, Interpolation::Linear),
    ])
}

fn canonical_unit_parameter(raw_parameter: f64) -> Option<f64> {
    if !raw_parameter.is_finite() {
        return None;
    }

    let parameter =
        (raw_parameter * HEIGHT_PARAMETER_KEY_SCALE).round() / HEIGHT_PARAMETER_KEY_SCALE;
    (0.0..=1.0).contains(&parameter).then_some(parameter)
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

fn quantize_m(value: f64) -> i64 {
    (value * HEIGHT_KEY_SCALE).round() as i64
}

impl NodeHeightPointKey {
    fn from_point(point: RoadVec2) -> Self {
        Self {
            x_mm: quantize_m(point.x),
            z_mm: quantize_m(point.y),
        }
    }
}

impl NodeHeightValueKey {
    fn from_height(height_m: f64) -> Self {
        Self(quantize_m(height_m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::input::NodeInputMouth;
    use crate::simulation::network::surface::ownership::NodeBooleanOwnership;
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
    fn rejects_same_xz_height_conflicts() {
        let input = conflicting_manual_input();
        let ownership = NodeBooleanOwnership {
            node_id: 77,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            footprint_shapes: Vec::new(),
            asphalt_shapes: Vec::new(),
            non_road_shapes: Vec::new(),
            owned_regions: vec![
                manual_region(RoadSurfaceBandKind::Carriageway, 0, 2.0),
                manual_region(RoadSurfaceBandKind::Sidewalk, 1, 5.0),
            ],
        };

        assert_eq!(
            NodeHeightSolution::from_ownership_and_input(&input, &ownership),
            Err(NodeHeightSourceError::SameXzHeightConflict {
                x_mm: 0,
                z_mm: 0,
                existing_height_mm: 2000,
                incoming_height_mm: 5000,
            })
        );
    }

    #[test]
    fn rejects_missing_source_band() {
        let input = conflicting_manual_input();
        let ownership = NodeBooleanOwnership {
            node_id: 77,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            footprint_shapes: Vec::new(),
            asphalt_shapes: Vec::new(),
            non_road_shapes: Vec::new(),
            owned_regions: vec![manual_region(RoadSurfaceBandKind::Carriageway, 99, 2.0)],
        };

        assert_eq!(
            NodeHeightSolution::from_ownership_and_input(&input, &ownership),
            Err(NodeHeightSourceError::MissingSourceBand {
                mouth_order_index: 0,
                band_index: 99,
            })
        );
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
        }
    }
}
