//! World-authoring variant export helpers.

use super::super::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn world_lake_fill_preview_dict(
        preview: Option<WorldLakeFillPreview>,
        ok: bool,
        message: &str,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("ok", ok);
        dict.set("message", GString::from(message));
        if let Some(preview) = preview {
            dict.set("active", true);
            dict.set("valid", preview.is_valid());
            dict.set("seed_world_x", f64::from(preview.seed_world_x));
            dict.set("seed_world_z", f64::from(preview.seed_world_z));
            dict.set("seed_height_m", f64::from(preview.seed_height_m));
            dict.set(
                "surface_elevation_m",
                f64::from(preview.surface_elevation_m),
            );
            dict.set("filled_cells", preview.filled_cells as i64);
            dict.set(
                "status",
                GString::from(match preview.status {
                    WorldLakeFillPreviewStatus::Ready => "ready",
                    WorldLakeFillPreviewStatus::SurfaceBelowSeedTerrain => "below_seed",
                    WorldLakeFillPreviewStatus::EscapesWorldEdge => "edge_escape",
                    WorldLakeFillPreviewStatus::DoesNotReachWorldEdge => "not_edge_connected",
                }),
            );
            dict.set(
                "kind",
                GString::from(match preview.kind {
                    WorldWaterFillKind::Lake => "lake",
                    WorldWaterFillKind::OpenWater => "open_water",
                }),
            );
        } else {
            dict.set("active", false);
            dict.set("valid", false);
            dict.set("filled_cells", 0_i64);
            dict.set("status", GString::from("inactive"));
            dict.set("kind", GString::from("inactive"));
        }
        dict
    }

    pub(in crate::nodes::simulation_node) fn world_water_authoring_marker_dict(
        kind: &str,
        world_x: f32,
        world_z: f32,
        terrain_height_m: f32,
        surface_elevation_m: Option<f32>,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("kind", GString::from(kind));
        dict.set("world_x", f64::from(world_x));
        dict.set("world_z", f64::from(world_z));
        dict.set("terrain_height_m", f64::from(terrain_height_m));
        if let Some(surface_elevation_m) = surface_elevation_m {
            dict.set("surface_elevation_m", f64::from(surface_elevation_m));
        }
        dict
    }
}
