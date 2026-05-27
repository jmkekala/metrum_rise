//! Incident mouth profile and path construction.

use super::classification::incident_direction_ordering;
use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_ordered_piece_mouths(
        &self,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<Vec<OrderedIncidentPieceMouth>> {
        let mut mouths = Vec::with_capacity(incidents.len());
        for &incident in incidents {
            let profile = self.build_incident_mouth_profile(incident)?;
            let endpoint_profile = self.build_incident_endpoint_profile(incident)?;
            let (
                boundary_paths_world,
                band_start_paths_world,
                band_end_paths_world,
                uses_explicit_band_domain_paths,
            ) = self.build_incident_mouth_paths(incident, &profile, &endpoint_profile);
            mouths.push(OrderedIncidentPieceMouth {
                profile,
                endpoint_profile,
                boundary_paths_world,
                band_start_paths_world,
                band_end_paths_world,
                uses_explicit_band_domain_paths,
                direction_angle_ccw: Self::normalized_angle_ccw(incident.direction_xz),
                direction_xz: incident.direction_xz,
                edge_idx: incident.edge_idx,
                side: incident.side,
            });
        }
        mouths.sort_by(|a, b| {
            incident_direction_ordering(
                a.direction_angle_ccw,
                a.edge_idx,
                a.side,
                b.direction_angle_ccw,
                b.edge_idx,
                b.side,
            )
        });
        Some(mouths)
    }

    fn build_incident_mouth_paths(
        &self,
        incident: IncidentSurfaceEdge,
        profile: &IncidentMouthProfile,
        endpoint_profile: &IncidentMouthProfile,
    ) -> (
        Vec<Vec<RoadVec3>>,
        Vec<Vec<RoadVec3>>,
        Vec<Vec<RoadVec3>>,
        bool,
    ) {
        let Some(sections) = self.compiled_sections.get(&incident.edge_idx) else {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        };
        let Some(mouth_index) = sections.iter().enumerate().find_map(|(index, section)| {
            let candidate = Self::build_mouth_profile_from_section(section, incident.side)?;
            incident_mouth_profiles_match(&candidate, profile).then_some(index)
        }) else {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        };

        let section_indices: Vec<usize> = match incident.side {
            IncidentEdgeSide::Start => (0..=mouth_index).rev().collect(),
            IncidentEdgeSide::End => (mouth_index..sections.len()).collect(),
        };
        let mut profile_path = Vec::with_capacity(section_indices.len());
        for section_index in section_indices {
            let Some(path_profile) =
                Self::build_mouth_profile_from_section(&sections[section_index], incident.side)
            else {
                return (Vec::new(), Vec::new(), Vec::new(), false);
            };
            if !incident_mouth_profiles_have_same_shape(profile, &path_profile) {
                return (Vec::new(), Vec::new(), Vec::new(), false);
            }
            profile_path.push(path_profile);
        }

        let Some(last_profile) = profile_path.last() else {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        };
        if !incident_mouth_profiles_match(last_profile, endpoint_profile) {
            return (Vec::new(), Vec::new(), Vec::new(), false);
        }

        let uses_explicit_band_domain_paths =
            incident_profile_path_has_non_collinear_center(&profile_path);
        let boundary_paths_world = (0..profile.boundary_points_world.len())
            .map(|boundary_index| {
                incident_world_path(
                    profile_path
                        .iter()
                        .map(|path_profile| path_profile.boundary_points_world[boundary_index]),
                )
            })
            .collect();
        let band_start_paths_world = (0..profile.bands.len())
            .map(|band_index| {
                incident_world_path(
                    profile_path
                        .iter()
                        .map(|path_profile| path_profile.bands[band_index].start_point_world),
                )
            })
            .collect();
        let band_end_paths_world = (0..profile.bands.len())
            .map(|band_index| {
                incident_world_path(
                    profile_path
                        .iter()
                        .map(|path_profile| path_profile.bands[band_index].end_point_world),
                )
            })
            .collect();
        (
            boundary_paths_world,
            band_start_paths_world,
            band_end_paths_world,
            uses_explicit_band_domain_paths,
        )
    }

    fn build_incident_mouth_profile(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentMouthProfile> {
        let piece = self.compiled_visual_span_pieces.get(&incident.edge_idx)?;
        match incident.side {
            IncidentEdgeSide::Start => piece.start_mouth_profile.clone(),
            IncidentEdgeSide::End => piece.end_mouth_profile.clone(),
        }
    }

    fn build_incident_endpoint_profile(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentMouthProfile> {
        let sections = self.compiled_sections.get(&incident.edge_idx)?;
        let section = match incident.side {
            IncidentEdgeSide::Start => sections.first()?,
            IncidentEdgeSide::End => sections.last()?,
        };
        Self::build_mouth_profile_from_section(section, incident.side)
    }
}

fn incident_mouth_profiles_match(
    left: &IncidentMouthProfile,
    right: &IncidentMouthProfile,
) -> bool {
    incident_mouth_profiles_have_same_shape(left, right)
        && left
            .boundary_points_world
            .iter()
            .zip(&right.boundary_points_world)
            .all(|(left, right)| {
                ArrangementBoundaryPointKey::from_world(*left)
                    == ArrangementBoundaryPointKey::from_world(*right)
            })
        && left.bands.iter().zip(&right.bands).all(|(left, right)| {
            ArrangementBoundaryPointKey::from_world(left.start_point_world)
                == ArrangementBoundaryPointKey::from_world(right.start_point_world)
                && ArrangementBoundaryPointKey::from_world(left.end_point_world)
                    == ArrangementBoundaryPointKey::from_world(right.end_point_world)
        })
}

fn incident_mouth_profiles_have_same_shape(
    left: &IncidentMouthProfile,
    right: &IncidentMouthProfile,
) -> bool {
    left.boundary_points_world.len() == right.boundary_points_world.len()
        && left.bands.len() == right.bands.len()
        && left
            .bands
            .iter()
            .zip(&right.bands)
            .all(|(left, right)| left.kind == right.kind)
}

fn incident_world_path(points: impl IntoIterator<Item = RoadVec3>) -> Vec<RoadVec3> {
    points.into_iter().collect()
}

fn incident_profile_path_has_non_collinear_center(path: &[IncidentMouthProfile]) -> bool {
    if path.len() <= 2 {
        return false;
    }
    let Some(start) = path.first().and_then(incident_profile_center_key) else {
        return false;
    };
    let Some(end) = path.last().and_then(incident_profile_center_key) else {
        return false;
    };
    if start == end {
        return false;
    }
    let dx = i128::from(end.x_key() - start.x_key());
    let dz = i128::from(end.z_key() - start.z_key());
    path[1..path.len() - 1].iter().any(|profile| {
        let Some(key) = incident_profile_center_key(profile) else {
            return false;
        };
        let px = i128::from(key.x_key() - start.x_key());
        let pz = i128::from(key.z_key() - start.z_key());
        px * dz - pz * dx != 0
    })
}

fn incident_profile_center_key(profile: &IncidentMouthProfile) -> Option<NodeArrangementKey> {
    let first = profile.boundary_points_world.first()?;
    let last = profile.boundary_points_world.last()?;
    let center = (*first + *last) * 0.5;
    Some(NodeArrangementKey::from_point(
        super::super::backend::road_vec3_xz(center),
    ))
}
