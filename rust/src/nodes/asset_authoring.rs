// SPDX-License-Identifier: GPL-2.0-only

//! Thin cached-catalog bridge for task-oriented building authoring; no live city state.

use crate::assets::authoring;
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, load_runtime_economy_catalog,
};
use godot::prelude::*;
use serde_json::{Value, json};
use std::sync::Arc;

/// Editor capability queries over a catalog loaded explicitly, not on every field edit.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct AssetAuthoringPolicy {
    catalog: Option<Arc<RuntimeEconomyCatalog>>,
    site_vertices: Vec<[f32; 2]>,
    inspection: Option<(String, GString)>,
}

#[godot_api]
impl AssetAuthoringPolicy {
    /// Resolve source-file paths when opening an exported asset or rebasing after publication.
    #[func]
    pub fn colour_sources_json(params: GString, directory: GString) -> GString {
        let result = serde_json::from_str::<Value>(&params.to_string())
            .map_err(|e| e.to_string())
            .and_then(|params| {
                authoring::colours::exported_sources(
                    &params,
                    std::path::Path::new(&directory.to_string()),
                )
            });
        match result {
            Ok(sources) => json!({"sources":sources}),
            Err(error) => json!({"error":error}),
        }
        .to_string()
        .as_str()
        .into()
    }

    /// Resolve an authored scheme into explicit preview bindings without changing geometry.
    #[func]
    pub fn colour_preview_json(document: GString, selected: GString) -> GString {
        let result = serde_json::from_str::<Value>(&document.to_string())
            .map_err(|e| e.to_string())
            .and_then(|state| authoring::colours::preview_plan(&state, &selected.to_string()));
        match result {
            Ok(plan) => json!({"plan":plan}),
            Err(error) => json!({"error":error}),
        }
        .to_string()
        .as_str()
        .into()
    }

    /// Inspect named source materials without loading or modifying preview geometry.
    #[func]
    pub fn colour_materials_json(path: GString) -> GString {
        let result = authoring::colours::materials(std::path::Path::new(&path.to_string()));
        let value = match result {
            Ok(materials) => json!({"materials": materials}),
            Err(error) => json!({"error": error}),
        };
        value.to_string().as_str().into()
    }

    /// List texture candidates for review only; this never edits the document.
    #[func]
    pub fn discover_colours_json(albedo: GString) -> GString {
        let result = authoring::colours::discover(std::path::Path::new(&albedo.to_string()));
        let value = match result {
            Ok(candidates) => json!({"candidates": candidates}),
            Err(error) => json!({"error": error}),
        };
        value.to_string().as_str().into()
    }

    /// List candidates for `albedo`'s base name inside another folder; review only.
    #[func]
    pub fn discover_colours_in_json(albedo: GString, folder: GString) -> GString {
        let albedo = std::path::PathBuf::from(albedo.to_string());
        let folder = folder.to_string();
        let value = match albedo.file_name() {
            // Publication keeps only the referenced albedo, so the alternatives still live
            // beside the original model rather than in the asset's own directory.
            Some(name) => {
                match authoring::colours::discover(&std::path::Path::new(&folder).join(name)) {
                    Ok(candidates) => json!({"candidates": candidates}),
                    Err(error) => json!({"error": error}),
                }
            }
            None => json!({"error": "Choose a source albedo texture first"}),
        };
        value.to_string().as_str().into()
    }

    /// Resolve an exact material name; missing and duplicate definitions are errors.
    #[func]
    pub fn colour_material_error(name: GString, inventory: GString) -> GString {
        let result =
            serde_json::from_str::<Vec<authoring::colours::MaterialSource>>(&inventory.to_string())
                .map_err(|error| error.to_string())
                .and_then(|materials| {
                    authoring::colours::resolve(&name.to_string(), &materials).map(|_| ())
                });
        result.err().unwrap_or_default().as_str().into()
    }

    /// Normalize interactive yaw using the shared deterministic cardinal snap rule.
    #[func]
    pub fn rotation_degrees(angle: f32) -> f32 {
        authoring::edits::rotation_degrees(angle)
    }
    /// Validate authored LOD metadata through the shared Rust manifest rules.
    #[func]
    pub fn lod_chain_error(lods: Array<VarDictionary>) -> GString {
        // Typed input retains NaN/infinity so invalid bounds cannot become JSON null/defaults.
        lods.iter_shared()
            .map(|entry| {
                let number = |value: Variant| {
                    value
                        .try_to::<f64>()
                        .map(|v| v as f32)
                        .map_err(|e| e.to_string())
                };
                Ok(crate::assets::LodEntry {
                    file: entry
                        .get("file")
                        .and_then(|v| v.try_to::<GString>().ok())
                        .unwrap_or_default()
                        .to_string(),
                    distance_min_m: number(entry.get("distance_min_m").unwrap_or_default())?,
                    distance_max_m: entry
                        .get("distance_max_m")
                        .filter(|v| !v.is_nil())
                        .map(number)
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, String>>()
            .and_then(|lods| authoring::lod_chain_error(&lods))
            .err()
            .unwrap_or_default()
            .as_str()
            .into()
    }

    /// Clamp a footprint's XZ translation into the lot; oversized axes remain centered.
    /// Constant work and no allocation, shared by mesh, anchor and yard manipulation.
    #[func]
    pub fn clamp_lot_translation(
        &self,
        bounds: Rect2,
        half_lot: Vector2,
        delta: Vector2,
    ) -> Vector2 {
        let end = bounds.end();
        let result = crate::assets::asset::geometry::clamp_translation(
            [bounds.position.x, bounds.position.y],
            [end.x, end.y],
            [half_lot.x, half_lot.y],
            [delta.x, delta.y],
        );
        Vector2::new(result[0], result[1])
    }

    /// Clamp an authored frontage coordinate, centering widths larger than the available edge.
    #[func]
    pub fn clamp_interval(&self, value: f32, min: f32, max: f32) -> f32 {
        crate::assets::asset::geometry::clamp_interval(value, min, max)
    }

    /// Check an editor-local polygon using the manifest validator's rules.
    /// O(V²) work and O(V) bridge conversion; scratch capacity is reused between edits.
    #[func]
    pub fn is_valid_site_polygon(&mut self, points: PackedVector2Array) -> bool {
        self.site_vertices.clear();
        self.site_vertices
            .extend(points.as_slice().iter().map(|p| [p.x, p.y]));
        crate::assets::asset::geometry::is_valid(&self.site_vertices)
    }

    /// Reload the runtime catalog; failures clear old data and return an actionable error.
    #[func]
    pub fn reload_catalog(&mut self) -> GString {
        self.inspection = None;
        match load_runtime_economy_catalog() {
            Ok(catalog) => {
                self.catalog = Some(catalog);
                GString::new()
            }
            Err(error) => {
                self.catalog = None;
                GString::from(error.as_str())
            }
        }
    }

    /// Return supported building presets and service subtypes as JSON.
    #[func]
    pub fn types_json(&self) -> GString {
        GString::from(authoring::types().to_string().as_str())
    }

    /// One read-only projection for applicability and diagnostics; reuse unchanged inspections.
    /// File availability remains an uncached boundary check and catalog reload invalidates this cache.
    #[func]
    pub fn inspect_json(&mut self, document: GString) -> GString {
        let source = document.to_string();
        if let Some((previous, result)) = &self.inspection
            && previous == &source
        {
            return result.clone();
        }
        let result = match serde_json::from_str::<Value>(&source) {
            Ok(data) if data.is_object() => {
                let descriptor = authoring::describe(&data, self.catalog.as_deref());
                let mut issues = authoring::issues(&data, self.catalog.as_deref(), &descriptor);
                let error =
                    crate::nodes::sim::asset_export::validate_asset_params_internal(&source);
                if !error.is_empty() {
                    issues.push(json!({"section": "validate", "field": "", "severity": "error", "message": error}));
                }
                json!({"issues": issues, "descriptor": descriptor})
            }
            _ => json!({"error": "Asset document must be a JSON object"}),
        };
        let result = GString::from(result.to_string().as_str());
        self.inspection = Some((source, result.clone()));
        result
    }

    /// Preview a conversion with an exact change list. Applying it requires UI confirmation.
    #[func]
    pub fn conversion_json(&self, document: GString, target: GString, subtype: GString) -> GString {
        let result = serde_json::from_str::<Value>(&document.to_string())
            .map_err(|error| error.to_string())
            .and_then(|data| {
                authoring::conversion(&data, &target.to_string(), &subtype.to_string())
            })
            .unwrap_or_else(|error| json!({"error": error}));
        GString::from(result.to_string().as_str())
    }
}
