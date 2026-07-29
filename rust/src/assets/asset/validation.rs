//! Structural and semantic asset manifest validation.

use super::{
    Anchor, AnchorType, AssetManifest, BuildingData, LodEntry, MeshPart, PlacementMode,
    SiteSurface, ZoneClass,
};
use crate::assets::{ManifestError, is_valid_asset_id};

const ZONE_CELL_M: f32 = 10.0;
const ANCHOR_FORWARD_UNIT_EPS: f32 = 0.02;
const ANCHOR_LOT_EPS_M: f32 = 0.001;
const ANCHOR_FRONTAGE_EDGE_EPS_M: f32 = 0.05;

impl AssetManifest {
    /// Validates structural and semantic constraints.
    ///
    /// Checks that:
    /// - `asset_id` is valid dot-separated kebab segments.
    /// - Exactly one class section is populated.
    /// - `display_name` is non-empty.
    /// - Building lot dimensions are non-zero when present.
    /// - Vehicle dimensions are positive when present.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if !is_valid_asset_id(&self.asset_id) {
            return Err(ManifestError::Validation(format!(
                "invalid asset_id '{}': must be dot-separated segments of lowercase \
                 letters, digits, and underscores",
                self.asset_id
            )));
        }
        if self.display_name.is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': display_name must not be empty",
                self.asset_id
            )));
        }

        // Enforce exactly one class section.
        let _ = self.class()?;

        for anchor in &self.anchors {
            validate_anchor_common(&self.asset_id, anchor)?;
        }

        if let Some(b) = &self.building {
            if !self.lods.is_empty() {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': building assets use [[mesh_parts]] with [[mesh_parts.lods]]; top-level [[lods]] is not valid",
                    self.asset_id
                )));
            }
            validate_building_mesh_parts(&self.asset_id, &self.mesh_parts)?;
            if b.lot_width_cells == 0 || b.lot_depth_cells == 0 {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': lot_width_cells and lot_depth_cells must be > 0",
                    self.asset_id
                )));
            }
            if b.level == 0 {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': level must be >= 1",
                    self.asset_id
                )));
            }
            if let Some(frontage_forward) = b.frontage_forward {
                validate_building_frontage_forward(&self.asset_id, frontage_forward)?;
            }
            let frontage_forward = self.building_frontage_forward();
            if b.effective_min_zone_width_cells() == 0 || b.effective_min_zone_depth_cells() == 0 {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': min_zone_width_cells and min_zone_depth_cells must be > 0",
                    self.asset_id
                )));
            }
            match b.placement_mode {
                PlacementMode::ZonedPrivate => {
                    if b.extractor.is_some() {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': extractor metadata requires explicit placement",
                            self.asset_id
                        )));
                    }
                    let Some(zone_type) = b.zone_type else {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': zoned_private buildings require zone_type",
                            self.asset_id
                        )));
                    };
                    let Some(density) = b.density_key() else {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': zoned_private buildings require density",
                            self.asset_id
                        )));
                    };
                    if density.trim().is_empty() {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': density must not be empty",
                            self.asset_id
                        )));
                    }
                    match zone_type {
                        ZoneClass::Residential => {
                            if b.household_capacity.is_none() {
                                return Err(ManifestError::Validation(format!(
                                    "asset_id '{}': residential zoned_private buildings require household_capacity",
                                    self.asset_id
                                )));
                            }
                            if b.worker_capacity.is_some() {
                                return Err(ManifestError::Validation(format!(
                                    "asset_id '{}': residential zoned_private buildings must not use worker_capacity",
                                    self.asset_id
                                )));
                            }
                        }
                        ZoneClass::Commercial | ZoneClass::Industrial => {
                            if b.worker_capacity.is_none() && b.economy_profile.is_none() {
                                return Err(ManifestError::Validation(format!(
                                    "asset_id '{}': commercial and industrial zoned_private buildings require worker_capacity or economy_profile",
                                    self.asset_id
                                )));
                            }
                        }
                        ZoneClass::Office | ZoneClass::Mixed => {
                            return Err(ManifestError::Validation(format!(
                                "asset_id '{}': office and mixed are reserved future extensions outside the baseline shipped building contract",
                                self.asset_id
                            )));
                        }
                    }
                }
                PlacementMode::Explicit => {
                    if b.zone_type.is_some() || b.density.is_some() {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': explicit buildings must not declare zone_type or density",
                            self.asset_id
                        )));
                    }
                    if let Some(extractor) = &b.extractor {
                        if extractor.resource.trim().is_empty() {
                            return Err(ManifestError::Validation(format!(
                                "asset_id '{}': building.extractor.resource must not be empty",
                                self.asset_id
                            )));
                        }
                        if extractor.area_mode.trim() != "player_polygon" {
                            return Err(ManifestError::Validation(format!(
                                "asset_id '{}': building.extractor.area_mode must be \"player_polygon\"",
                                self.asset_id
                            )));
                        }
                        if b.economy_profile.is_none() {
                            return Err(ManifestError::Validation(format!(
                                "asset_id '{}': extractor buildings require economy_profile",
                                self.asset_id
                            )));
                        }
                    }
                }
            }

            let mut main_entrance_count = 0usize;
            for anchor in &self.anchors {
                validate_building_anchor_position(&self.asset_id, b, anchor)?;
                match anchor.anchor_type {
                    AnchorType::Entrance if anchor.name == "main" => {
                        main_entrance_count += 1;
                    }
                    AnchorType::Entrance => {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': additional entrance anchor '{}' is not allowed on building assets; use type = \"driveway\", \"parking\", or \"loading_bay\" for site access points",
                            self.asset_id, anchor.name
                        )));
                    }
                    AnchorType::Driveway => {
                        validate_positive_anchor_field(
                            &self.asset_id,
                            anchor,
                            "width_m",
                            anchor.width_m,
                        )?;
                        validate_building_driveway_frontage_edge(
                            &self.asset_id,
                            b,
                            anchor,
                            frontage_forward,
                        )?;
                        validate_building_site_anchor_footprint(&self.asset_id, b, anchor)?;
                    }
                    AnchorType::Parking | AnchorType::LoadingBay => {
                        validate_positive_anchor_field(
                            &self.asset_id,
                            anchor,
                            "width_m",
                            anchor.width_m,
                        )?;
                        validate_positive_anchor_field(
                            &self.asset_id,
                            anchor,
                            "length_m",
                            anchor.length_m,
                        )?;
                        validate_building_site_anchor_footprint(&self.asset_id, b, anchor)?;
                    }
                    AnchorType::Wheel => {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': anchor '{}' uses type = \"wheel\", which is not valid for building assets",
                            self.asset_id, anchor.name
                        )));
                    }
                    AnchorType::Light => {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': anchor '{}' uses type = \"light\", which is not valid for building assets",
                            self.asset_id, anchor.name
                        )));
                    }
                }
            }
            if main_entrance_count != 1 {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': building assets require exactly one [[anchors]] entry with type = \"entrance\" and name = \"main\"",
                    self.asset_id
                )));
            }
            for surface in &self.site_surfaces {
                validate_building_site_surface(&self.asset_id, b, surface)?;
            }
        } else if !self.mesh_parts.is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': [[mesh_parts]] is only valid for building assets",
                self.asset_id
            )));
        } else if !self.site_surfaces.is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': [[site_surfaces]] is only valid for building assets",
                self.asset_id
            )));
        }

        if let Some(v) = &self.vehicle
            && (v.length_m <= 0.0 || v.width_m <= 0.0 || v.height_m <= 0.0)
        {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': vehicle dimensions must be positive",
                self.asset_id
            )));
        }

        Ok(())
    }
}

fn validate_positive_anchor_field(
    asset_id: &str,
    anchor: &Anchor,
    field_name: &str,
    value: Option<f32>,
) -> Result<(), ManifestError> {
    let Some(value) = value else {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' with type = \"{:?}\" requires positive {}",
            asset_id, anchor.name, anchor.anchor_type, field_name
        )));
    };
    if !value.is_finite() || value <= 0.0 {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' has invalid {} {}; expected a positive finite value",
            asset_id, anchor.name, field_name, value
        )));
    }
    Ok(())
}

fn validate_anchor_common(asset_id: &str, anchor: &Anchor) -> Result<(), ManifestError> {
    validate_finite_vec3(asset_id, anchor, "position", anchor.position)?;
    validate_anchor_forward(asset_id, anchor)?;
    if let Some(vehicle_class) = anchor.vehicle_class.as_deref() {
        validate_anchor_vehicle_class(asset_id, anchor, vehicle_class)?;
    }
    Ok(())
}

fn validate_finite_vec3(
    asset_id: &str,
    anchor: &Anchor,
    field_name: &str,
    value: [f32; 3],
) -> Result<(), ManifestError> {
    if value.iter().any(|component| !component.is_finite()) {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' has invalid {} {:?}; expected finite values",
            asset_id, anchor.name, field_name, value
        )));
    }
    Ok(())
}

fn validate_anchor_forward(asset_id: &str, anchor: &Anchor) -> Result<(), ManifestError> {
    validate_finite_vec3(asset_id, anchor, "forward", anchor.forward)?;
    let [x, y, z] = anchor.forward;
    let length = (x * x + y * y + z * z).sqrt();
    if length <= ANCHOR_FORWARD_UNIT_EPS || (length - 1.0).abs() > ANCHOR_FORWARD_UNIT_EPS {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' forward {:?} must be a non-zero unit vector",
            asset_id, anchor.name, anchor.forward
        )));
    }
    Ok(())
}

fn validate_building_frontage_forward(
    asset_id: &str,
    forward: [f32; 3],
) -> Result<(), ManifestError> {
    if forward.iter().any(|component| !component.is_finite()) {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': building frontage_forward {:?} must contain finite values",
            asset_id, forward
        )));
    }
    let [x, y, z] = forward;
    let length = (x * x + y * y + z * z).sqrt();
    let horizontal_length = (x * x + z * z).sqrt();
    if length <= ANCHOR_FORWARD_UNIT_EPS
        || (length - 1.0).abs() > ANCHOR_FORWARD_UNIT_EPS
        || horizontal_length <= ANCHOR_FORWARD_UNIT_EPS
        || y.abs() > ANCHOR_FORWARD_UNIT_EPS
    {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': building frontage_forward {:?} must be a horizontal unit vector",
            asset_id, forward
        )));
    }
    Ok(())
}

fn validate_anchor_vehicle_class(
    asset_id: &str,
    anchor: &Anchor,
    vehicle_class: &str,
) -> Result<(), ManifestError> {
    match vehicle_class.trim() {
        "car" | "freight" | "service" => Ok(()),
        other => Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' has invalid vehicle_class '{}'; expected car, freight, or service",
            asset_id, anchor.name, other
        ))),
    }
}

fn validate_building_anchor_position(
    asset_id: &str,
    building: &BuildingData,
    anchor: &Anchor,
) -> Result<(), ManifestError> {
    let half_width = f32::from(building.lot_width_cells) * ZONE_CELL_M * 0.5;
    let half_depth = f32::from(building.lot_depth_cells) * ZONE_CELL_M * 0.5;
    let [x, _, z] = anchor.position;
    if x.abs() > half_width + ANCHOR_LOT_EPS_M || z.abs() > half_depth + ANCHOR_LOT_EPS_M {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' position [{}, {}, {}] is outside the building lot bounds +/-{}m x +/-{}m",
            asset_id,
            anchor.name,
            anchor.position[0],
            anchor.position[1],
            anchor.position[2],
            half_width,
            half_depth
        )));
    }
    Ok(())
}

fn validate_building_site_anchor_footprint(
    asset_id: &str,
    building: &BuildingData,
    anchor: &Anchor,
) -> Result<(), ManifestError> {
    let (anchor_width, anchor_length) = match anchor.anchor_type {
        AnchorType::Driveway => {
            let width = anchor.width_m.unwrap_or(0.0);
            (width, (width * 1.4).max(1.5))
        }
        AnchorType::Parking | AnchorType::LoadingBay => (
            anchor.width_m.unwrap_or(0.0),
            anchor.length_m.unwrap_or(0.0),
        ),
        _ => return Ok(()),
    };
    let [forward_x, _, forward_z] = anchor.forward;
    let horizontal_len = (forward_x * forward_x + forward_z * forward_z).sqrt();
    if horizontal_len <= ANCHOR_FORWARD_UNIT_EPS {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' with type = \"{:?}\" must have a horizontal forward direction",
            asset_id, anchor.name, anchor.anchor_type
        )));
    }
    let fwd_x = forward_x / horizontal_len;
    let fwd_z = forward_z / horizontal_len;
    let side_x = -fwd_z;
    let side_z = fwd_x;
    let half_anchor_width = anchor_width * 0.5;
    let half_lot_width = f32::from(building.lot_width_cells) * ZONE_CELL_M * 0.5;
    let half_lot_depth = f32::from(building.lot_depth_cells) * ZONE_CELL_M * 0.5;
    let offsets = [
        (-side_x * half_anchor_width, -side_z * half_anchor_width),
        (side_x * half_anchor_width, side_z * half_anchor_width),
        (
            side_x * half_anchor_width + fwd_x * anchor_length,
            side_z * half_anchor_width + fwd_z * anchor_length,
        ),
        (
            -side_x * half_anchor_width + fwd_x * anchor_length,
            -side_z * half_anchor_width + fwd_z * anchor_length,
        ),
    ];
    for (offset_x, offset_z) in offsets {
        let x = anchor.position[0] + offset_x;
        let z = anchor.position[2] + offset_z;
        if x.abs() > half_lot_width + ANCHOR_LOT_EPS_M
            || z.abs() > half_lot_depth + ANCHOR_LOT_EPS_M
        {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': anchor '{}' site footprint for type = \"{:?}\" crosses the building lot bounds +/-{}m x +/-{}m",
                asset_id, anchor.name, anchor.anchor_type, half_lot_width, half_lot_depth
            )));
        }
    }
    Ok(())
}

fn validate_building_driveway_frontage_edge(
    asset_id: &str,
    building: &BuildingData,
    anchor: &Anchor,
    frontage_forward: [f32; 3],
) -> Result<(), ManifestError> {
    let half_lot_width = f32::from(building.lot_width_cells) * ZONE_CELL_M * 0.5;
    let half_lot_depth = f32::from(building.lot_depth_cells) * ZONE_CELL_M * 0.5;
    let Some((edge_axis_x, outward_sign)) = building_cardinal_frontage_axis(frontage_forward)
    else {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': driveway anchors require cardinal building frontage_forward, got {:?}",
            asset_id, frontage_forward
        )));
    };
    let edge_distance = if edge_axis_x {
        half_lot_width
    } else {
        half_lot_depth
    };
    let edge_position = outward_sign * edge_distance;
    let anchor_edge_position = if edge_axis_x {
        anchor.position[0]
    } else {
        anchor.position[2]
    };
    if (anchor_edge_position - edge_position).abs() > ANCHOR_FRONTAGE_EDGE_EPS_M {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': driveway anchor '{}' must lie on the road-facing frontage edge at {}={}m",
            asset_id,
            anchor.name,
            if edge_axis_x { "x" } else { "z" },
            edge_position
        )));
    }

    let [forward_x, _, forward_z] = anchor.forward;
    let horizontal_len = (forward_x * forward_x + forward_z * forward_z).sqrt();
    if horizontal_len <= ANCHOR_FORWARD_UNIT_EPS {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': driveway anchor '{}' must point inward from the frontage edge",
            asset_id, anchor.name
        )));
    }
    let inward_x = if edge_axis_x { -outward_sign } else { 0.0 };
    let inward_z = if edge_axis_x { 0.0 } else { -outward_sign };
    let inward_dot =
        (forward_x / horizontal_len) * inward_x + (forward_z / horizontal_len) * inward_z;
    if inward_dot < 1.0 - ANCHOR_FORWARD_UNIT_EPS {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': driveway anchor '{}' must point inward from the frontage edge",
            asset_id, anchor.name
        )));
    }
    Ok(())
}

fn building_cardinal_frontage_axis(frontage_forward: [f32; 3]) -> Option<(bool, f32)> {
    let [x, _, z] = frontage_forward;
    if x.abs() >= 1.0 - ANCHOR_FORWARD_UNIT_EPS && z.abs() <= ANCHOR_FORWARD_UNIT_EPS {
        return Some((true, if x >= 0.0 { 1.0 } else { -1.0 }));
    }
    if z.abs() >= 1.0 - ANCHOR_FORWARD_UNIT_EPS && x.abs() <= ANCHOR_FORWARD_UNIT_EPS {
        return Some((false, if z >= 0.0 { 1.0 } else { -1.0 }));
    }
    None
}

fn validate_building_site_surface(
    asset_id: &str,
    building: &BuildingData,
    surface: &SiteSurface,
) -> Result<(), ManifestError> {
    if !surface.y_m.is_finite() {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': site surface '{}' has invalid y_m {}; expected a finite value",
            asset_id, surface.name, surface.y_m
        )));
    }
    if surface.vertices.len() < 3 {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': site surface '{}' must contain at least three vertices",
            asset_id, surface.name
        )));
    }
    let half_lot_width = f32::from(building.lot_width_cells) * ZONE_CELL_M * 0.5;
    let half_lot_depth = f32::from(building.lot_depth_cells) * ZONE_CELL_M * 0.5;

    for (vertex_index, [x, z]) in surface.vertices.iter().copied().enumerate() {
        if !x.is_finite() || !z.is_finite() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': site surface '{}' vertex {} has invalid coordinate [{}, {}]; expected finite values",
                asset_id, surface.name, vertex_index, x, z
            )));
        }
        if x.abs() > half_lot_width + ANCHOR_LOT_EPS_M
            || z.abs() > half_lot_depth + ANCHOR_LOT_EPS_M
        {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': site surface '{}' vertex {} crosses the building lot bounds +/-{}m x +/-{}m",
                asset_id, surface.name, vertex_index, half_lot_width, half_lot_depth
            )));
        }
    }

    if site_surface_polygon_signed_area(&surface.vertices).abs() <= 0.001 {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': site surface '{}' has zero or near-zero polygon area",
            asset_id, surface.name
        )));
    }

    if site_surface_polygon_self_intersects(&surface.vertices) {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': site surface '{}' polygon self-intersects",
            asset_id, surface.name
        )));
    }

    Ok(())
}

fn site_surface_polygon_signed_area(vertices: &[[f32; 2]]) -> f32 {
    let mut twice_area = 0.0;
    for i in 0..vertices.len() {
        let [ax, az] = vertices[i];
        let [bx, bz] = vertices[(i + 1) % vertices.len()];
        twice_area += ax * bz - bx * az;
    }
    twice_area * 0.5
}

fn site_surface_polygon_self_intersects(vertices: &[[f32; 2]]) -> bool {
    for a in 0..vertices.len() {
        let b = (a + 1) % vertices.len();
        for c in (a + 1)..vertices.len() {
            let d = (c + 1) % vertices.len();
            if a == c || a == d || b == c || b == d {
                continue;
            }
            if site_surface_segments_intersect(vertices[a], vertices[b], vertices[c], vertices[d]) {
                return true;
            }
        }
    }
    false
}

fn site_surface_segments_intersect(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> bool {
    const EPS: f32 = 0.0001;
    let ab_c = site_surface_orientation(a, b, c);
    let ab_d = site_surface_orientation(a, b, d);
    let cd_a = site_surface_orientation(c, d, a);
    let cd_b = site_surface_orientation(c, d, b);

    if ab_c.abs() <= EPS && site_surface_point_on_segment(a, b, c) {
        return true;
    }
    if ab_d.abs() <= EPS && site_surface_point_on_segment(a, b, d) {
        return true;
    }
    if cd_a.abs() <= EPS && site_surface_point_on_segment(c, d, a) {
        return true;
    }
    if cd_b.abs() <= EPS && site_surface_point_on_segment(c, d, b) {
        return true;
    }

    (ab_c > EPS) != (ab_d > EPS) && (cd_a > EPS) != (cd_b > EPS)
}

fn site_surface_orientation(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn site_surface_point_on_segment(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> bool {
    const EPS: f32 = 0.0001;
    p[0] >= a[0].min(b[0]) - EPS
        && p[0] <= a[0].max(b[0]) + EPS
        && p[1] >= a[1].min(b[1]) - EPS
        && p[1] <= a[1].max(b[1]) + EPS
}

fn validate_building_mesh_parts(
    asset_id: &str,
    mesh_parts: &[MeshPart],
) -> Result<(), ManifestError> {
    if mesh_parts.is_empty() {
        return Err(ManifestError::Validation(format!(
            "asset_id '{asset_id}': building assets require at least one [[mesh_parts]] entry"
        )));
    }

    let mut names: Vec<&str> = Vec::with_capacity(mesh_parts.len());
    for (part_index, part) in mesh_parts.iter().enumerate() {
        let name = part.name.trim();
        if name.is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': mesh part {part_index} name must not be empty"
            )));
        }
        if names.contains(&name) {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': duplicate mesh part name '{name}'"
            )));
        }
        names.push(name);
        if part.scale <= 0.0 || !part.scale.is_finite() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': mesh part '{name}' scale must be finite and > 0"
            )));
        }
        if part.rotation_degrees[0].abs() > 1e-4 || part.rotation_degrees[2].abs() > 1e-4 {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': mesh part '{name}' only supports Y rotation in the building renderer"
            )));
        }
        validate_lods(asset_id, Some(name), &part.lods)?;
    }

    Ok(())
}

fn validate_lods(
    asset_id: &str,
    part_name: Option<&str>,
    lods: &[LodEntry],
) -> Result<(), ManifestError> {
    if lods.is_empty() {
        let owner = part_name
            .map(|name| format!("mesh part '{name}'"))
            .unwrap_or_else(|| "asset".to_owned());
        return Err(ManifestError::Validation(format!(
            "asset_id '{asset_id}': {owner} requires at least one LOD entry"
        )));
    }

    let mut previous_min = -f32::INFINITY;
    for (lod_index, lod) in lods.iter().enumerate() {
        if lod.file.trim().is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': LOD {lod_index} file must not be empty"
            )));
        }
        if !lod.distance_min_m.is_finite() || lod.distance_min_m < 0.0 {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': LOD {lod_index} distance_min_m must be finite and >= 0"
            )));
        }
        if lod.distance_min_m < previous_min {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': LOD entries must be ordered by distance_min_m"
            )));
        }
        if let Some(max) = lod.distance_max_m
            && (!max.is_finite() || max <= lod.distance_min_m)
        {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': LOD {lod_index} distance_max_m must be finite and greater than distance_min_m"
            )));
        }
        previous_min = lod.distance_min_m;
    }

    Ok(())
}
