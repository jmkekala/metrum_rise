//! dedicated background thread. The render thread reads only from a
//! `RenderSnapshot` that the sim thread writes after each tick.
//!
//! ### Method Mapping
//!
//! | Category | Method | Godot Caller |
//! |----------|--------|--------------|
//! | **System** | `undo_action` | `input_manager.gd` |
//! | | `create_blank_world` | future world-editor UI |
//! | | `save_game` | `input_manager.gd`, `main.gd` |
//! | | `load_game` | `input_manager.gd`, `main.gd` |
//! | | `save_world_definition` | future world-editor UI |
//! | | `load_world_definition` | future world-editor UI |
//! | | `get_perf_stats` | `debug_panel.gd` |
//! | | `get_demand_pressures` | `main_ui.gd` |
//! | | `get_treasury_balance` | `main_ui.gd` |
//! | | `get_agent_count` | `main_ui.gd` |
//! | **Economy Editor** | `is_economy_editor_mode` | `economy_editor.gd` |
//! | | `load_economy_project` | `economy_editor.gd` |
//! | | `export_economy_project` | `economy_editor.gd` |
//! | | `run_economy_sandbox` | `economy_editor.gd` |
//! | **World Editor** | `is_world_editor_mode` | `world_editor.gd` |
//! | | `create_blank_world` | `world_editor.gd` |
//! | | `save_world_definition` | `world_editor.gd` |
//! | | `load_world_definition` | `world_editor.gd` |
//! | | `begin_world_lake_fill_preview` | `world_editor.gd` |
//! | | `update_world_lake_fill_preview` | `world_editor.gd` |
//! | | `get_world_lake_fill_preview` | `world_editor.gd` |
//! | | `commit_world_lake_fill_preview` | `world_editor.gd` |
//! | | `cancel_world_lake_fill_preview` | `world_editor.gd` |
//! | **Environment** | `get_pollution_image_data` | `overlay_manager.gd` |
//! | | `get_noise_image_data` | `overlay_manager.gd` |
//! | | `get_desirability_image_data` | `overlay_manager.gd` |
//! | **Terrain** | `sculpt_terrain` | `terrain_tool.gd` |
//! | | `smooth_terrain` | `world_editor.gd` |
//! | | `slope_terrain` | `world_editor.gd` |
//! | | `is_terrain_dirty` | `terrain.gd` |
//! | | `clear_terrain_dirty` | `terrain.gd` |
//! | | `get_terrain_patch_layout` | `terrain.gd`, `water.gd` |
//! | | `get_dirty_terrain_patches` | `terrain.gd` |
//! | | `get_terrain_patch` | `terrain.gd` |
//! | | `get_terrain_border_loop` | `terrain.gd`, `water.gd` |
//! | | `get_height_at` | `road_tool.gd`, `building_tool.gd` |
//! | | `intersect_terrain` | `input_manager.gd` (mouse pick) |
//! | | `get_world_surface_height` | `road_tool.gd`, `move_tool.gd` |
//! | | `intersect_world_surface` | `road_tool.gd`, `select_tool.gd` |
//! | **Water** | `add_water` | `water_tool.gd` |
//! | | `add_water_source` | `water_tool.gd` |
//! | | `is_water_dirty` | `water.gd` |
//! | | `clear_water_dirty` | `water.gd` |
//! | | `get_dirty_water_patches` | `water.gd` |
//! | | `get_water_patch` | `water.gd` |
//! | | `get_water_patch_debug` | `water.gd` |
//! | | `get_water_patch_authored_fill_debug` | `water.gd` |
//! | | `get_water_border_depths` | `water.gd` |
//! | **Network** | `add_road` | `road_tool.gd` |
//! | | `is_network_dirty` | `network_renderer.gd` |
//! | | `clear_network_dirty` | `network_renderer.gd` |
//! | | `rebuild_network_surface_terrain` | `network_renderer.gd` |
//! | | `get_road_mesh_data` | `network_renderer.gd` |
//! | | `get_preview_road_surface` | `road_tool.gd` |
//! | | `get_road_surface_debug_data` | `network_tool.gd` |
//! | | `get_closest_network_point` | `road_tool.gd`, `zoning_tool.gd` |
//! | | `check_border_candidate` | `road_tool.gd` |
//! | | `set_border_connection` | `road_tool.gd` |
//! | **Zoning** | `get_zone_profiles` | `zoning_tool.gd`, `asset_editor.gd` |
//! | | `capture_zoning_patch` | `zoning_tool.gd` |
//! | | `apply_zoning_patch` | `zoning_tool.gd` |
//! | | `restore_zoning_patch` | `zoning_tool.gd` |
//! | | `get_zone_profile_texture_data_rg8` | `zoning_overlay.gd` |
//! | | `get_zone_profile_style_lut_rgba8` | `zoning_overlay.gd` |
//! | **Agents** | `get_agent_transforms` | `agent_renderer.gd` |
//! | | `get_car_transforms` | `agent_renderer.gd` |
//! | | `set_camera_aabb` | `agents.gd` (culling update) |

use godot::classes::{INode3D, Node3D};
use godot::prelude::*;

use crate::config;
use crate::nodes::sim::core::{CityTreasury, RenderSnapshot, SimCommand, SimCore, run_sim_thread};
use crate::nodes::sim::core::{
    WorldLakeFillPreview, WorldLakeFillPreviewStatus, WorldWaterFillKind,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::load_runtime_economy_tuning;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::ZoningSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtError, TerrainCdtInput, TerrainCdtPatch, TerrainCdtRoadLoop, TerrainCdtVertex,
    build_road_touched_terrain_patch,
};
use crate::simulation::water::WaterSystem;

use crate::debug_log;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};

#[derive(GodotClass)]
#[class(base=Node3D)]
/// The central simulation node exposed to Godot.
///
/// All simulation state is in [`SimCore`] behind an `Arc<Mutex<>>`. This struct
/// holds only Godot-facing fields plus the handles needed to communicate with the
/// background sim thread.
pub struct SimulationNode {
    /// All simulation state — ticked by the background thread.
    pub(crate) core: Arc<Mutex<SimCore>>,
    /// Latest pre-computed rendering data from the sim thread.
    pub(crate) snapshot: Arc<RwLock<RenderSnapshot>>,
    /// Background sim thread handle.
    pub(crate) sim_thread: Option<std::thread::JoinHandle<()>>,
    /// Channel to send commands (speed changes, quit) to the sim thread.
    pub(crate) cmd_tx: std::sync::mpsc::Sender<SimCommand>,
    /// Receiver held here until `ready()` transfers it to the background thread.
    pub(crate) cmd_rx: Option<std::sync::mpsc::Receiver<SimCommand>>,
    /// True when running in headless benchmark mode.
    pub(crate) benchmark_mode: bool,
    /// True when launched with `--asset-editor`. Sim thread is not started;
    /// the node runs a 500 m sandbox for preview only.
    pub(crate) asset_editor_mode: bool,
    /// True when launched with `--economy-editor`. Sim thread is not started;
    /// the node only serves the authored-economy editor shell.
    pub(crate) economy_editor_mode: bool,
    /// True when launched with `--world-editor`. The node boots the blank-world
    /// authoring shell and keeps the simulation thread available for terrain /
    /// future water authoring workflows.
    pub(crate) world_editor_mode: bool,
    /// Incremented every Godot frame in benchmark mode.
    pub(crate) benchmark_tick_count: u32,
    /// Last day for which benchmark CSV has been written.
    pub(crate) last_logged_day: u32,
    /// Accumulated Godot render time (unused by sim, kept for potential UI use).
    pub(crate) time_passed: f64,
    base: Base<Node3D>,
}

struct RoadClipPolygonQuery {
    polygons: Vec<crate::simulation::network::surface::RoadSurfaceVisualPolygon>,
    source_count: usize,
}

impl SimulationNode {
    // ── Lifecycle ──

    /// Acquires the sim-core mutex, recovering silently if it was poisoned by a
    /// prior sim-thread panic.  Using `unwrap()` on a poisoned mutex would
    /// crash Godot on the next frame even though the sim thread has already
    /// recovered; this helper matches the recovery logic in `run_sim_thread`.
    #[inline]
    fn lock_core(&self) -> std::sync::MutexGuard<'_, crate::nodes::sim::core::SimCore> {
        match self.core.lock() {
            Ok(g) => g,
            Err(e) => {
                godot_error!("[sim] mutex poisoned — recovering in Godot main-thread call");
                e.into_inner()
            }
        }
    }

    /// Non-blocking mutex acquire. Returns `None` if the sim thread currently holds
    /// the lock (e.g., during `add_road_internal`). Used for per-frame read-only
    /// calls (terrain raycast, network snap) so the Godot main thread never stalls
    /// waiting for an in-progress road placement.
    #[inline]
    fn try_lock_core(&self) -> Option<std::sync::MutexGuard<'_, crate::nodes::sim::core::SimCore>> {
        match self.core.try_lock() {
            Ok(g) => Some(g),
            Err(std::sync::TryLockError::WouldBlock) => None,
            Err(std::sync::TryLockError::Poisoned(e)) => {
                godot_error!("[sim] mutex poisoned — recovering in try_lock_core");
                Some(e.into_inner())
            }
        }
    }

    /// Returns the dimensions of the heightmap.
    pub fn get_heightmap_size_internal(&self) -> Vector2 {
        let core = self.lock_core();
        Vector2::new(core.heightmap.width as f32, core.heightmap.height as f32)
    }

    /// Spawns the background simulation thread.
    fn start_sim_thread(&mut self) {
        if let Some(rx) = self.cmd_rx.take() {
            let core = Arc::clone(&self.core);
            let snap = Arc::clone(&self.snapshot);
            self.sim_thread = Some(std::thread::spawn(move || {
                run_sim_thread(core, snap, rx);
            }));
        }
    }

    /// Rebuilds the render snapshot immediately from the current core state.
    fn refresh_snapshot_from_core(&self) {
        let snapshot = {
            let core = self.lock_core();
            core.build_snapshot()
        };
        *self.snapshot.write().unwrap() = snapshot;
    }

    fn world_lake_fill_preview_dict(
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

    fn world_water_authoring_marker_dict(
        kind: &str,
        world_x: f32,
        world_z: f32,
        terrain_height_m: f32,
        surface_elevation_m: Option<f32>,
        rate_m_per_tick: Option<f32>,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("kind", GString::from(kind));
        dict.set("world_x", f64::from(world_x));
        dict.set("world_z", f64::from(world_z));
        dict.set("terrain_height_m", f64::from(terrain_height_m));
        if let Some(surface_elevation_m) = surface_elevation_m {
            dict.set("surface_elevation_m", f64::from(surface_elevation_m));
        }
        if let Some(rate_m_per_tick) = rate_m_per_tick {
            dict.set("rate_m_per_tick", f64::from(rate_m_per_tick));
        }
        dict
    }

    fn terrain_patch_dict(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("patch_x", i64::try_from(patch.patch_x).unwrap_or(0));
        dict.set("patch_z", i64::try_from(patch.patch_z).unwrap_or(0));
        dict.set(
            "sample_width",
            i64::try_from(patch.sample_width).unwrap_or(0),
        );
        dict.set(
            "sample_height",
            i64::try_from(patch.sample_height).unwrap_or(0),
        );
        dict.set(
            "texture_width",
            i64::try_from(patch.texture_width).unwrap_or(0),
        );
        dict.set(
            "texture_height",
            i64::try_from(patch.texture_height).unwrap_or(0),
        );
        dict.set(
            "inner_offset_x",
            i64::try_from(patch.inner_offset_x).unwrap_or(0),
        );
        dict.set(
            "inner_offset_z",
            i64::try_from(patch.inner_offset_z).unwrap_or(0),
        );
        dict.set("world_origin_x", f64::from(patch.world_origin_x));
        dict.set("world_origin_z", f64::from(patch.world_origin_z));
        dict.set("world_size_x", f64::from(patch.world_size_x));
        dict.set("world_size_z", f64::from(patch.world_size_z));
        dict.set(
            "height_data",
            PackedFloat32Array::from_iter(patch.height_data.iter().copied()),
        );
        dict
    }

    fn refined_terrain_patch_dict(
        core: &crate::nodes::sim::core::SimCore,
        patch_x: usize,
        patch_z: usize,
        render_step_m: f32,
    ) -> VarDictionary {
        let Some(base_patch) = core.heightmap.visual_patch_snapshot(patch_x, patch_z) else {
            return VarDictionary::new();
        };

        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let sample_width = if base_patch.world_size_x <= f32::EPSILON {
            1
        } else {
            ((base_patch.world_size_x / safe_render_step_m).round() as usize).max(1) + 1
        };
        let sample_height = if base_patch.world_size_z <= f32::EPSILON {
            1
        } else {
            ((base_patch.world_size_z / safe_render_step_m).round() as usize).max(1) + 1
        };
        let texture_width = sample_width + base_patch.inner_offset_x * 2;
        let texture_height = sample_height + base_patch.inner_offset_z * 2;
        let mut height_data = Vec::with_capacity(texture_width * texture_height);
        for local_z in 0..texture_height {
            let clamped_inner_z = if local_z < base_patch.inner_offset_z {
                0
            } else if local_z >= base_patch.inner_offset_z + sample_height {
                sample_height.saturating_sub(1)
            } else {
                local_z - base_patch.inner_offset_z
            };
            let sample_t_z = if sample_height <= 1 {
                0.0
            } else {
                clamped_inner_z as f32 / sample_height.saturating_sub(1) as f32
            };
            let world_z = base_patch.world_origin_z + sample_t_z * base_patch.world_size_z;

            for local_x in 0..texture_width {
                let clamped_inner_x = if local_x < base_patch.inner_offset_x {
                    0
                } else if local_x >= base_patch.inner_offset_x + sample_width {
                    sample_width.saturating_sub(1)
                } else {
                    local_x - base_patch.inner_offset_x
                };
                let sample_t_x = if sample_width <= 1 {
                    0.0
                } else {
                    clamped_inner_x as f32 / sample_width.saturating_sub(1) as f32
                };
                let world_x = base_patch.world_origin_x + sample_t_x * base_patch.world_size_x;
                height_data.push(core.heightmap.sample_visual_height_world(world_x, world_z));
            }
        }

        let refined_patch = crate::simulation::terrain::TerrainPatchSnapshot {
            patch_x,
            patch_z,
            sample_width,
            sample_height,
            texture_width,
            texture_height,
            inner_offset_x: base_patch.inner_offset_x,
            inner_offset_z: base_patch.inner_offset_z,
            world_origin_x: base_patch.world_origin_x,
            world_origin_z: base_patch.world_origin_z,
            world_size_x: base_patch.world_size_x,
            world_size_z: base_patch.world_size_z,
            height_data,
        };
        let mut dict = Self::terrain_patch_dict(&refined_patch);
        let road_clip_query = Self::road_clip_polygon_query_for_bounds(
            core,
            base_patch.world_origin_x,
            base_patch.world_origin_z,
            base_patch.world_origin_x + base_patch.world_size_x,
            base_patch.world_origin_z + base_patch.world_size_z,
        );
        Self::append_road_clip_polygons(&mut dict, &road_clip_query.polygons);
        Self::append_cdt_terrain_mesh(
            &mut dict,
            &refined_patch,
            &road_clip_query.polygons,
            road_clip_query.source_count > 0,
            true,
        );
        dict
    }

    fn append_road_clip_polygons_for_bounds(
        dict: &mut VarDictionary,
        core: &SimCore,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) {
        let road_clip_polygons =
            Self::road_clip_polygons_for_bounds(core, min_x, min_z, max_x, max_z);
        Self::append_road_clip_polygons(dict, &road_clip_polygons);
    }

    fn road_clip_polygons_for_bounds(
        core: &SimCore,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<crate::simulation::network::surface::RoadSurfaceVisualPolygon> {
        Self::road_clip_polygon_query_for_bounds(core, min_x, min_z, max_x, max_z).polygons
    }

    fn road_clip_polygon_query_for_bounds(
        core: &SimCore,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> RoadClipPolygonQuery {
        let (polygons, source_count) = core
            .transit_network
            .road_surface
            .terrain_clip_polygons_and_source_count_for_world_bounds(
                &core.region_graph,
                min_x,
                min_z,
                max_x,
                max_z,
            );
        RoadClipPolygonQuery {
            polygons,
            source_count,
        }
    }

    fn append_road_clip_polygons(
        dict: &mut VarDictionary,
        road_clip_polygons: &[crate::simulation::network::surface::RoadSurfaceVisualPolygon],
    ) {
        if road_clip_polygons.is_empty() {
            return;
        }

        let mut polygon_counts = Vec::with_capacity(road_clip_polygons.len());
        let mut polygon_points = Vec::new();
        for polygon in road_clip_polygons {
            polygon_counts.push(i32::try_from(polygon.points_world.len()).unwrap_or(0));
            polygon_points.extend(polygon.points_world.iter().copied());
        }
        dict.set(
            "road_clip_polygon_counts",
            PackedInt32Array::from_iter(polygon_counts),
        );
        dict.set(
            "road_clip_polygon_points",
            PackedVector3Array::from_iter(polygon_points),
        );
    }

    fn append_cdt_terrain_mesh(
        dict: &mut VarDictionary,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_clip_polygons: &[crate::simulation::network::surface::RoadSurfaceVisualPolygon],
        has_grounded_road_contributors: bool,
        requires_road_clip: bool,
    ) {
        if road_clip_polygons.is_empty() {
            if has_grounded_road_contributors || requires_road_clip {
                Self::append_empty_cdt_failure(dict, "missing_road_clip_polygons");
            }
            return;
        }

        match build_road_touched_terrain_patch(Self::terrain_cdt_input(patch, road_clip_polygons)) {
            Ok(mesh) => {
                let cdt_status = if mesh.stats.invalid_constraint_edges == 0 {
                    "ok"
                } else {
                    "conflicted"
                };
                dict.set("terrain_cdt_status", GString::from(cdt_status));
                dict.set(
                    "terrain_cdt_input_vertices",
                    i64::try_from(mesh.stats.input_vertices).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_constraint_edges",
                    i64::try_from(mesh.stats.constraint_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_road_constraint_edges",
                    i64::try_from(mesh.stats.road_constraint_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_accepted_faces",
                    i64::try_from(mesh.stats.accepted_faces).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_rejected_road_faces",
                    i64::try_from(mesh.stats.rejected_road_faces).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_preserved_road_constraint_edges",
                    i64::try_from(mesh.stats.preserved_road_constraint_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_invalid_constraints",
                    i64::try_from(mesh.stats.invalid_constraint_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_max_face_y_delta_m",
                    f64::from(mesh.stats.max_face_y_delta_m),
                );
                dict.set(
                    "terrain_cdt_max_face_slope_ratio",
                    f64::from(mesh.stats.max_face_slope_ratio),
                );
                dict.set(
                    "terrain_cdt_road_seam_faces",
                    i64::try_from(mesh.stats.road_seam_faces).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_road_seam_steep_faces",
                    i64::try_from(mesh.stats.road_seam_steep_faces).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_road_seam_max_y_delta_m",
                    f64::from(mesh.stats.road_seam_max_y_delta_m),
                );
                dict.set(
                    "terrain_cdt_road_seam_max_slope_ratio",
                    f64::from(mesh.stats.road_seam_max_slope_ratio),
                );
                Self::append_cdt_road_seam_face_samples(dict, &mesh);
                Self::append_cdt_invalid_constraint_samples(dict, &mesh);
                Self::append_cdt_mesh_buffers(dict, patch, &mesh);
            }
            Err(err) => {
                Self::append_empty_cdt_failure(dict, Self::terrain_cdt_error_label(&err));
            }
        }
    }

    fn append_empty_cdt_failure(dict: &mut VarDictionary, error_label: &'static str) {
        dict.set("terrain_cdt_status", GString::from("failed"));
        dict.set("terrain_cdt_error", GString::from(error_label));
        dict.set("terrain_cdt_input_vertices", 0i64);
        dict.set("terrain_cdt_constraint_edges", 0i64);
        dict.set("terrain_cdt_road_constraint_edges", 0i64);
        dict.set("terrain_cdt_accepted_faces", 0i64);
        dict.set("terrain_cdt_rejected_road_faces", 0i64);
        dict.set("terrain_cdt_preserved_road_constraint_edges", 0i64);
        dict.set("terrain_cdt_invalid_constraints", 1i64);
        dict.set("terrain_cdt_emitted_faces", 0i64);
        dict.set("terrain_cdt_max_face_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_max_face_slope_ratio", 0.0f64);
        dict.set("terrain_cdt_road_seam_faces", 0i64);
        dict.set("terrain_cdt_road_seam_steep_faces", 0i64);
        dict.set("terrain_cdt_road_seam_max_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_road_seam_max_slope_ratio", 0.0f64);
        dict.set(
            "terrain_cdt_road_seam_sample_centroids",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_bounds",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_metrics",
            PackedFloat32Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_vertices",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_invalid_constraint_sample_edges",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_invalid_constraint_sample_metadata",
            PackedInt32Array::new(),
        );
        dict.set("terrain_mesh_vertices", PackedVector3Array::new());
        dict.set("terrain_mesh_normals", PackedVector3Array::new());
        dict.set("terrain_mesh_uvs", PackedVector2Array::new());
    }

    fn terrain_cdt_input(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_clip_polygons: &[crate::simulation::network::surface::RoadSurfaceVisualPolygon],
    ) -> TerrainCdtInput {
        let patch_model = TerrainCdtPatch::new(
            f64::from(patch.world_origin_x),
            f64::from(patch.world_origin_z),
            f64::from(patch.world_origin_x + patch.world_size_x),
            f64::from(patch.world_origin_z + patch.world_size_z),
            [
                Self::terrain_patch_sample_height_m(patch, 0, 0),
                Self::terrain_patch_sample_height_m(
                    patch,
                    0,
                    patch.sample_height.saturating_sub(1),
                ),
                Self::terrain_patch_sample_height_m(
                    patch,
                    patch.sample_width.saturating_sub(1),
                    patch.sample_height.saturating_sub(1),
                ),
                Self::terrain_patch_sample_height_m(patch, patch.sample_width.saturating_sub(1), 0),
            ],
        );
        let road_loops = road_clip_polygons
            .iter()
            .enumerate()
            .map(|(index, polygon)| {
                TerrainCdtRoadLoop::new(
                    u64::try_from(index).unwrap_or(u64::MAX),
                    0,
                    polygon
                        .points_world
                        .iter()
                        .map(|point| {
                            TerrainCdtVertex::new(f64::from(point.x), point.y, f64::from(point.z))
                        })
                        .collect(),
                )
            })
            .collect();

        let mut source_samples =
            Vec::with_capacity(patch.sample_width.saturating_mul(patch.sample_height));
        for sample_z in 0..patch.sample_height {
            let world_z = Self::terrain_patch_sample_world_z(patch, sample_z);
            for sample_x in 0..patch.sample_width {
                source_samples.push(TerrainCdtVertex::new(
                    f64::from(Self::terrain_patch_sample_world_x(patch, sample_x)),
                    Self::terrain_patch_sample_height_m(patch, sample_x, sample_z),
                    f64::from(world_z),
                ));
            }
        }

        TerrainCdtInput::new(patch_model, road_loops, source_samples)
    }

    fn terrain_patch_sample_world_x(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        sample_x: usize,
    ) -> f32 {
        if patch.sample_width <= 1 {
            patch.world_origin_x
        } else {
            patch.world_origin_x
                + patch.world_size_x * sample_x as f32 / patch.sample_width.saturating_sub(1) as f32
        }
    }

    fn terrain_patch_sample_world_z(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        sample_z: usize,
    ) -> f32 {
        if patch.sample_height <= 1 {
            patch.world_origin_z
        } else {
            patch.world_origin_z
                + patch.world_size_z * sample_z as f32
                    / patch.sample_height.saturating_sub(1) as f32
        }
    }

    fn terrain_patch_sample_height_m(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        sample_x: usize,
        sample_z: usize,
    ) -> f32 {
        if patch.texture_width == 0 || patch.height_data.is_empty() {
            return 0.0;
        }
        let texture_x = patch
            .inner_offset_x
            .saturating_add(sample_x.min(patch.sample_width.saturating_sub(1)));
        let texture_z = patch
            .inner_offset_z
            .saturating_add(sample_z.min(patch.sample_height.saturating_sub(1)));
        let index = texture_z
            .saturating_mul(patch.texture_width)
            .saturating_add(texture_x)
            .min(patch.height_data.len().saturating_sub(1));
        patch.height_data[index] * config::HEIGHT_SCALE
    }

    fn append_cdt_mesh_buffers(
        dict: &mut VarDictionary,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let center_x = patch.world_origin_x + patch.world_size_x * 0.5;
        let center_z = patch.world_origin_z + patch.world_size_z * 0.5;
        let mut vertices = Vec::with_capacity(mesh.triangles.len() * 3);
        let mut normals = Vec::with_capacity(mesh.triangles.len() * 3);
        let mut uvs = Vec::with_capacity(mesh.triangles.len() * 3);
        let mut emitted_faces = 0usize;

        for triangle in &mesh.triangles {
            let mut points = [
                Self::terrain_cdt_vertex_to_vector3(mesh.vertices[triangle[0]]),
                Self::terrain_cdt_vertex_to_vector3(mesh.vertices[triangle[1]]),
                Self::terrain_cdt_vertex_to_vector3(mesh.vertices[triangle[2]]),
            ];
            let mut raw_normal = (points[1] - points[0]).cross(points[2] - points[0]);
            if raw_normal.length_squared() <= 0.000_001 {
                continue;
            }
            if raw_normal.y < 0.0 {
                points.swap(1, 2);
                raw_normal = (points[1] - points[0]).cross(points[2] - points[0]);
            }
            let normal = raw_normal.normalized();
            emitted_faces += 1;
            for point in points {
                vertices.push(Vector3::new(
                    point.x - center_x,
                    point.y,
                    point.z - center_z,
                ));
                normals.push(normal);
                uvs.push(Vector2::new(
                    ((point.x - patch.world_origin_x) / patch.world_size_x.max(0.001))
                        .clamp(0.0, 1.0),
                    ((point.z - patch.world_origin_z) / patch.world_size_z.max(0.001))
                        .clamp(0.0, 1.0),
                ));
            }
        }

        dict.set(
            "terrain_cdt_emitted_faces",
            i64::try_from(emitted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_mesh_vertices",
            PackedVector3Array::from_iter(vertices),
        );
        dict.set(
            "terrain_mesh_normals",
            PackedVector3Array::from_iter(normals),
        );
        dict.set("terrain_mesh_uvs", PackedVector2Array::from_iter(uvs));
    }

    fn append_cdt_road_seam_face_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut centroids = Vec::with_capacity(mesh.road_seam_face_samples.len());
        let mut bounds = Vec::with_capacity(mesh.road_seam_face_samples.len() * 2);
        let mut metrics = Vec::with_capacity(mesh.road_seam_face_samples.len() * 2);
        let mut vertices = Vec::with_capacity(mesh.road_seam_face_samples.len() * 3);
        for sample in &mesh.road_seam_face_samples {
            centroids.push(Self::terrain_cdt_vertex_to_vector3(sample.centroid));
            bounds.push(Vector3::new(
                sample.min_x as f32,
                sample.min_y_m,
                sample.min_z as f32,
            ));
            bounds.push(Vector3::new(
                sample.max_x as f32,
                sample.max_y_m,
                sample.max_z as f32,
            ));
            metrics.push(sample.max_y_delta_m);
            metrics.push(sample.max_slope_ratio);
            vertices.extend(
                sample
                    .vertices
                    .into_iter()
                    .map(Self::terrain_cdt_vertex_to_vector3),
            );
        }
        dict.set(
            "terrain_cdt_road_seam_sample_centroids",
            PackedVector3Array::from_iter(centroids),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_bounds",
            PackedVector3Array::from_iter(bounds),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_metrics",
            PackedFloat32Array::from_iter(metrics),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_vertices",
            PackedVector3Array::from_iter(vertices),
        );
    }

    fn append_cdt_invalid_constraint_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut edges = Vec::with_capacity(mesh.invalid_constraint_samples.len() * 2);
        let mut metadata = Vec::with_capacity(mesh.invalid_constraint_samples.len() * 4);
        for sample in &mesh.invalid_constraint_samples {
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.start));
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.end));
            metadata.push(if sample.road_owned { 1 } else { 0 });
            metadata.push(i32::try_from(sample.stable_piece_id).unwrap_or(i32::MAX));
            metadata.push(i32::try_from(sample.local_loop_index).unwrap_or(i32::MAX));
            metadata.push(i32::try_from(sample.local_edge_index).unwrap_or(i32::MAX));
        }
        dict.set(
            "terrain_cdt_invalid_constraint_sample_edges",
            PackedVector3Array::from_iter(edges),
        );
        dict.set(
            "terrain_cdt_invalid_constraint_sample_metadata",
            PackedInt32Array::from_iter(metadata),
        );
    }

    fn terrain_cdt_vertex_to_vector3(vertex: TerrainCdtVertex) -> Vector3 {
        Vector3::new(vertex.x as f32, vertex.height_m, vertex.z as f32)
    }

    fn terrain_cdt_error_label(err: &TerrainCdtError) -> &'static str {
        match err {
            TerrainCdtError::InvalidPatch => "invalid_patch",
            TerrainCdtError::TriangulationFailed => "triangulation_failed",
        }
    }

    fn water_patch_dict(patch: &crate::simulation::water::WaterPatchSnapshot) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("patch_x", i64::try_from(patch.patch_x).unwrap_or(0));
        dict.set("patch_z", i64::try_from(patch.patch_z).unwrap_or(0));
        dict.set(
            "sample_width",
            i64::try_from(patch.sample_width).unwrap_or(0),
        );
        dict.set(
            "sample_height",
            i64::try_from(patch.sample_height).unwrap_or(0),
        );
        dict.set(
            "texture_width",
            i64::try_from(patch.texture_width).unwrap_or(0),
        );
        dict.set(
            "texture_height",
            i64::try_from(patch.texture_height).unwrap_or(0),
        );
        dict.set(
            "inner_offset_x",
            i64::try_from(patch.inner_offset_x).unwrap_or(0),
        );
        dict.set(
            "inner_offset_z",
            i64::try_from(patch.inner_offset_z).unwrap_or(0),
        );
        dict.set("world_origin_x", f64::from(patch.world_origin_x));
        dict.set("world_origin_z", f64::from(patch.world_origin_z));
        dict.set("world_size_x", f64::from(patch.world_size_x));
        dict.set("world_size_z", f64::from(patch.world_size_z));
        dict.set(
            "depth_data",
            PackedFloat32Array::from_iter(patch.depth_data.iter().copied()),
        );
        dict.set(
            "velocity_data",
            PackedFloat32Array::from_iter(patch.velocity_data.iter().copied()),
        );
        dict
    }

    fn water_patch_layer_debug_dict(
        stats: &crate::simulation::water::WaterPatchLayerStats,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "total_samples",
            i64::try_from(stats.total_samples).unwrap_or(0),
        );
        dict.set(
            "baseline_nonzero",
            i64::try_from(stats.baseline_nonzero).unwrap_or(0),
        );
        dict.set("baseline_max", f64::from(stats.baseline_max));
        dict.set("baseline_sum", f64::from(stats.baseline_sum));
        dict.set(
            "dynamic_nonzero",
            i64::try_from(stats.dynamic_nonzero).unwrap_or(0),
        );
        dict.set("dynamic_max", f64::from(stats.dynamic_max));
        dict.set("dynamic_sum", f64::from(stats.dynamic_sum));
        dict.set(
            "combined_nonzero",
            i64::try_from(stats.combined_nonzero).unwrap_or(0),
        );
        dict.set("combined_max", f64::from(stats.combined_max));
        dict.set("combined_sum", f64::from(stats.combined_sum));
        dict.set(
            "velocity_nonzero",
            i64::try_from(stats.velocity_nonzero).unwrap_or(0),
        );
        dict.set("velocity_max", f64::from(stats.velocity_max));
        dict.set("velocity_sum", f64::from(stats.velocity_sum));
        dict.set(
            "source_count_in_patch",
            i64::try_from(stats.source_count_in_patch).unwrap_or(0),
        );
        dict.set("source_rate_sum", f64::from(stats.source_rate_sum));
        dict.set("source_rate_abs_sum", f64::from(stats.source_rate_abs_sum));
        dict.set(
            "source_count_total",
            i64::try_from(stats.source_count_total).unwrap_or(0),
        );
        dict
    }

    fn authored_water_patch_fill_debug_dict(
        fill: &crate::nodes::sim::world_definition::AuthoredWaterPatchFillDebug,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "kind",
            GString::from(match fill.kind {
                WorldWaterFillKind::Lake => "lake",
                WorldWaterFillKind::OpenWater => "open_water",
            }),
        );
        dict.set("fill_index", i64::from(fill.fill_index));
        dict.set("preview", fill.preview);
        dict.set("world_x", f64::from(fill.world_x));
        dict.set("world_z", f64::from(fill.world_z));
        dict.set("surface_elevation_m", f64::from(fill.surface_elevation_m));
        dict.set(
            "filled_cells",
            i64::try_from(fill.filled_cells).unwrap_or(0),
        );
        dict.set("touches_world_edge", fill.touches_world_edge);
        dict.set(
            "patch_nonzero_samples",
            i64::try_from(fill.patch_nonzero_samples).unwrap_or(0),
        );
        dict.set("patch_max_depth_m", f64::from(fill.patch_max_depth_m));
        dict.set("patch_sum_depth_m", f64::from(fill.patch_sum_depth_m));
        dict
    }
}

#[godot_api]
impl SimulationNode {
    // ── Environment ──

    /// Returns the pollution image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_pollution_image_data(&self) -> PackedByteArray {
        self.lock_core().get_pollution_image_data_internal()
    }

    /// Returns the noise image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_noise_image_data(&self) -> PackedByteArray {
        self.lock_core().get_noise_image_data_internal()
    }

    /// Returns the desirability image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_desirability_image_data(&self) -> PackedByteArray {
        self.lock_core().get_desirability_image_data_internal()
    }

    // ── System ──

    /// Undoes the last action.
    #[func]
    pub fn undo_action(&mut self) -> bool {
        self.lock_core().undo_action_internal()
    }

    // ── Terrain & Water ──

    /// Sculpts the terrain heightmap.
    #[func]
    pub fn sculpt_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .sculpt_terrain_internal(pos, radius, strength);
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Begins one batched world-editor terrain stroke.
    #[func]
    pub fn begin_terrain_stroke(&mut self) {
        self.lock_core().start_terrain_stroke_internal();
    }

    /// Finalizes one batched world-editor terrain stroke.
    #[func]
    pub fn end_terrain_stroke(&mut self) -> bool {
        let (ended, terrain_dirty, water_dirty, network_dirty) = {
            let mut core = self.lock_core();
            let ended = core.end_terrain_stroke_internal();
            (
                ended,
                core.terrain_dirty,
                core.water_dirty,
                core.network_dirty,
            )
        };
        if ended {
            let mut snapshot = self.snapshot.write().unwrap();
            snapshot.terrain_dirty = terrain_dirty;
            snapshot.water_dirty = water_dirty;
            snapshot.network_dirty = network_dirty;
        }
        ended
    }

    /// Applies one batched terrain sculpt step during an active editor stroke.
    #[func]
    pub fn sculpt_terrain_stroke_step(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .sculpt_terrain_stroke_step_internal(pos, radius, strength);
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Moves terrain toward a clicked rendered heightmap level.
    #[func]
    pub fn level_terrain(
        &mut self,
        pos: Vector2,
        radius: f32,
        target_height_m: f32,
        strength: f32,
    ) {
        self.lock_core()
            .level_terrain_internal(pos, radius, target_height_m, strength);
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Applies one batched terrain-level step during an active editor stroke.
    #[func]
    pub fn level_terrain_stroke_step(
        &mut self,
        pos: Vector2,
        radius: f32,
        target_height_m: f32,
        strength: f32,
    ) {
        self.lock_core()
            .level_terrain_stroke_step_internal(pos, radius, target_height_m, strength);
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Smooths terrain toward the local neighborhood average.
    #[func]
    pub fn smooth_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .smooth_terrain_internal(pos, radius, strength);
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Applies one batched terrain-smooth step during an active editor stroke.
    #[func]
    pub fn smooth_terrain_stroke_step(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .smooth_terrain_stroke_step_internal(pos, radius, strength);
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Moves terrain toward a slope defined by two clicked rendered anchor points.
    #[func]
    pub fn slope_terrain(
        &mut self,
        pos: Vector2,
        radius: f32,
        start_world: Vector2,
        start_height_m: f32,
        end_world: Vector2,
        end_height_m: f32,
        strength: f32,
    ) {
        self.lock_core().slope_terrain_internal(
            pos,
            radius,
            start_world,
            start_height_m,
            end_world,
            end_height_m,
            strength,
        );
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Applies one batched terrain-slope step during an active editor stroke.
    #[func]
    pub fn slope_terrain_stroke_step(
        &mut self,
        pos: Vector2,
        radius: f32,
        start_world: Vector2,
        start_height_m: f32,
        end_world: Vector2,
        end_height_m: f32,
        strength: f32,
    ) {
        self.lock_core().slope_terrain_stroke_step_internal(
            pos,
            radius,
            start_world,
            start_height_m,
            end_world,
            end_height_m,
            strength,
        );
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Adds a volume of water at a specific grid position.
    #[func]
    pub fn add_water(&mut self, pos: Vector2, amount: f32) {
        self.lock_core().add_water_internal(pos, amount);
    }

    /// Adds a continuous water source at a specific grid position.
    #[func]
    pub fn add_water_source(&mut self, pos: Vector2, rate_add: f32) {
        self.lock_core().add_water_source_internal(pos, rate_add);
    }

    /// Returns whether the terrain mesh needs rebuilding.
    #[func]
    pub fn is_terrain_dirty(&self) -> bool {
        self.snapshot.read().unwrap().terrain_dirty
    }

    /// Returns whether the water mesh needs rebuilding.
    #[func]
    pub fn is_water_dirty(&self) -> bool {
        self.snapshot.read().unwrap().water_dirty
    }

    /// Clears the terrain dirty flag.
    #[func]
    pub fn clear_terrain_dirty(&mut self) {
        self.lock_core().terrain_dirty = false;
        self.lock_core().heightmap.clear_dirty_render_patches();
        self.snapshot.write().unwrap().terrain_dirty = false;
    }

    /// Clears the water dirty flag.
    #[func]
    pub fn clear_water_dirty(&mut self) {
        self.lock_core().water_dirty = false;
        self.lock_core().watermap.clear_dirty_render_patches();
        self.snapshot.write().unwrap().water_dirty = false;
    }

    /// Returns true if the road/rail network was mutated and the visual mesh needs a rebuild.
    ///
    /// `NetworkRenderer._process` polls this each frame. The flag stays `true` until
    /// `clear_network_dirty()` is called by GDScript after the refresh is complete,
    /// matching the same explicit-clear pattern used by `terrain_dirty` and `water_dirty`.
    #[func]
    pub fn is_network_dirty(&self) -> bool {
        self.snapshot.read().unwrap().network_dirty
    }

    /// Clears the network-dirty flag after `NetworkRenderer` has rebuilt the road/rail mesh.
    #[func]
    pub fn clear_network_dirty(&mut self) {
        self.lock_core().network_dirty = false;
        self.snapshot.write().unwrap().network_dirty = false;
    }

    /// Returns render-patch layout metadata shared by terrain and water renderers.
    #[func]
    pub fn get_terrain_patch_layout(&self) -> VarDictionary {
        let core = self.lock_core();
        let mut dict = VarDictionary::new();
        dict.set(
            "patch_cols",
            i64::try_from(core.heightmap.render_patch_cols()).unwrap_or(0),
        );
        dict.set(
            "patch_rows",
            i64::try_from(core.heightmap.render_patch_rows()).unwrap_or(0),
        );
        dict.set(
            "patch_interval_cells",
            i64::try_from(core.heightmap.render_patch_interval_cells()).unwrap_or(0),
        );
        dict.set("terrain_cell_m", f64::from(core.config.terrain_cell_m));
        dict.set("chunk_span_m", f64::from(core.heightmap.chunk_span_m()));
        dict
    }

    /// Returns the currently dirty terrain render patches as flat `(x, z)` pairs.
    #[func]
    pub fn get_dirty_terrain_patches(&self) -> PackedInt32Array {
        let core = self.lock_core();
        let mut patches: Vec<(usize, usize)> = core
            .heightmap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        patches.sort_unstable();
        let mut packed = PackedInt32Array::new();
        for (patch_x, patch_z) in patches {
            packed.push(i32::try_from(patch_x).unwrap_or(i32::MAX));
            packed.push(i32::try_from(patch_z).unwrap_or(i32::MAX));
        }
        packed
    }

    /// Returns one visual-terrain render patch, including its one-sample border ring.
    #[func]
    pub fn get_terrain_patch(&self, patch_x: i32, patch_z: i32) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        let Some(patch) = core.heightmap.visual_patch_snapshot(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        Self::terrain_patch_dict(&patch)
    }

    /// Returns one visible-terrain render patch resampled at a finer render step.
    #[func]
    pub fn get_refined_terrain_patch(
        &self,
        patch_x: i32,
        patch_z: i32,
        render_step_m: f32,
    ) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        Self::refined_terrain_patch_dict(&core, patch_x, patch_z, render_step_m)
    }

    /// Returns the terrain-border perimeter loop as world-space top positions.
    #[func]
    pub fn get_terrain_border_loop(&self) -> PackedVector3Array {
        PackedVector3Array::from_iter(self.lock_core().heightmap.border_loop_positions())
    }

    /// Returns the currently dirty water render patches as flat `(x, z)` pairs.
    #[func]
    pub fn get_dirty_water_patches(&self) -> PackedInt32Array {
        let core = self.lock_core();
        let mut patches: Vec<(usize, usize)> = core
            .watermap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        patches.sort_unstable();
        let mut packed = PackedInt32Array::new();
        for (patch_x, patch_z) in patches {
            packed.push(i32::try_from(patch_x).unwrap_or(i32::MAX));
            packed.push(i32::try_from(patch_z).unwrap_or(i32::MAX));
        }
        packed
    }

    /// Returns one visible-water render patch, including its one-sample border ring.
    #[func]
    pub fn get_water_patch(&self, patch_x: i32, patch_z: i32) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        let Some(patch) = core.watermap.visible_patch_snapshot(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        let mut dict = Self::water_patch_dict(&patch);
        Self::append_road_clip_polygons_for_bounds(
            &mut dict,
            &core,
            patch.world_origin_x,
            patch.world_origin_z,
            patch.world_origin_x + patch.world_size_x,
            patch.world_origin_z + patch.world_size_z,
        );
        dict
    }

    /// Returns debug-only baseline/dynamic/combined water stats for one render patch.
    #[func]
    pub fn get_water_patch_debug(&self, patch_x: i32, patch_z: i32) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        let Some(stats) = core.watermap.visible_patch_layer_stats(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        Self::water_patch_layer_debug_dict(&stats)
    }

    /// Returns debug-only authored baseline-water fill contributors for one render patch.
    #[func]
    pub fn get_water_patch_authored_fill_debug(&self, patch_x: i32, patch_z: i32) -> VarArray {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarArray::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarArray::new();
        };
        let core = self.lock_core();
        let contributors = core.authored_water_patch_fill_debug_internal(patch_x, patch_z);
        let mut array = VarArray::new();
        for contributor in contributors {
            let dict = Self::authored_water_patch_fill_debug_dict(&contributor);
            array.push(&dict.to_variant());
        }
        array
    }

    /// Returns the visible water depth along the world-edge perimeter loop.
    #[func]
    pub fn get_water_border_depths(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.lock_core().watermap.border_loop_depths())
    }

    /// Returns the dimensions of the heightmap.
    #[func]
    pub fn get_heightmap_size(&self) -> Vector2 {
        self.get_heightmap_size_internal()
    }

    /// Returns the terrain world extent in metres.
    #[func]
    pub fn get_terrain_world_size(&self) -> Vector2 {
        self.lock_core().get_terrain_world_size_internal()
    }

    // ── Zoning ──

    /// Returns the validated runtime zoning-profile registry for Godot tools and UI.
    #[func]
    pub fn get_zone_profiles(&self) -> VarArray {
        let core = self.lock_core();
        let mut arr = VarArray::new();
        for profile in core.zoning.profiles.profiles() {
            let mut dict = VarDictionary::new();
            dict.set("id", GString::from(profile.id.as_str()));
            dict.set("runtime_id", i64::from(profile.runtime_id));
            dict.set("display_name", GString::from(profile.display_name.as_str()));
            dict.set("ui_order", i64::from(profile.ui_order));
            dict.set("zone_type", GString::from(profile.zone_type.as_str()));
            dict.set("density", GString::from(profile.density.as_str()));
            dict.set(
                "ui_color",
                GString::from(
                    format!(
                        "#{:02X}{:02X}{:02X}",
                        profile.ui_color_rgb[0], profile.ui_color_rgb[1], profile.ui_color_rgb[2]
                    )
                    .as_str(),
                ),
            );
            dict.set("ui_icon", GString::from(profile.ui_icon.as_str()));
            dict.set(
                "ui_description",
                GString::from(profile.ui_description.as_str()),
            );
            arr.push(&dict.to_variant());
        }
        arr
    }

    /// Captures one zoning patch bounding box as packed little-endian runtime ids.
    #[func]
    pub fn capture_zoning_patch(
        &self,
        grid_x: i32,
        grid_y: i32,
        width_cells: i32,
        height_cells: i32,
    ) -> PackedByteArray {
        PackedByteArray::from_iter(self.lock_core().capture_zoning_patch_internal(
            grid_x,
            grid_y,
            width_cells,
            height_cells,
        ))
    }

    /// Applies one masked zoning paint patch.
    #[func]
    pub fn apply_zoning_patch(
        &mut self,
        grid_x: i32,
        grid_y: i32,
        width_cells: i32,
        height_cells: i32,
        target_profile_runtime_id: i32,
        write_mask: PackedByteArray,
    ) {
        self.lock_core().apply_zoning_patch_internal(
            grid_x,
            grid_y,
            width_cells,
            height_cells,
            target_profile_runtime_id,
            write_mask.to_vec(),
        );
    }

    /// Restores one full zoning patch bounding box from packed little-endian runtime ids.
    #[func]
    pub fn restore_zoning_patch(
        &mut self,
        grid_x: i32,
        grid_y: i32,
        width_cells: i32,
        height_cells: i32,
        profile_ids_le_u16: PackedByteArray,
    ) {
        self.lock_core().restore_zoning_patch_internal(
            grid_x,
            grid_y,
            width_cells,
            height_cells,
            profile_ids_le_u16.to_vec(),
        );
    }

    /// Returns the authoritative zoning-profile grid as RG8 bytes for texture upload.
    #[func]
    pub fn get_zone_profile_texture_data_rg8(&self) -> PackedByteArray {
        PackedByteArray::from_iter(self.lock_core().zoning.get_zone_profile_texture_data_rg8())
    }

    /// Returns the one-row RGBA8 style LUT for the profile-aware zoning overlay.
    #[func]
    pub fn get_zone_profile_style_lut_rgba8(&self) -> PackedByteArray {
        PackedByteArray::from_iter(self.lock_core().zoning.get_zone_profile_style_lut_rgba8())
    }

    /// Returns the occupied grid as a flat byte array (0/1 per cell) for texture upload.
    #[func]
    pub fn get_occupied_texture_data(&self) -> PackedByteArray {
        PackedByteArray::from_iter(self.lock_core().zoning.get_occupied_texture_data())
    }

    /// Returns the distance-to-road grid as a flat byte array for texture upload.
    #[func]
    pub fn get_distance_texture_data(&self) -> PackedByteArray {
        PackedByteArray::from_iter(self.lock_core().zoning.get_distance_texture_data())
    }

    /// Returns the no-build mask as a flat `u8` byte array (0 or 255 per cell).
    ///
    /// Cells within ~32 m of a `no_building_spawn` road surface are 255; all others are 0.
    /// Upload as an R8 texture to drive shader-side zone-tint suppression on no-build roads.
    #[func]
    pub fn get_no_build_mask_texture_data(&self) -> PackedByteArray {
        PackedByteArray::from_iter(self.lock_core().zoning.get_no_build_mask_texture_data())
    }

    /// Returns the ID of the edge hovered by the mouse.
    #[func]
    pub fn get_hovered_edge(&self, world_x: f32, world_z: f32) -> i32 {
        self.lock_core().get_hovered_edge_internal(world_x, world_z)
    }

    /// Returns the raycast depth against the road network.
    #[func]
    pub fn get_max_polygon_depth(
        &self,
        origin_x: f32,
        origin_z: f32,
        dir_x: f32,
        dir_z: f32,
        max_search: f32,
    ) -> f32 {
        self.lock_core()
            .get_max_polygon_depth_internal(origin_x, origin_z, dir_x, dir_z, max_search)
    }

    // ── Simulation ──

    /// Sets the simulation speed multiplier.
    #[func]
    pub fn set_simulation_speed(&mut self, speed: f32) {
        // Use channel so we don't block waiting for the tick lock.
        let _ = self.cmd_tx.send(SimCommand::SetSpeed(speed.max(0.0)));
    }

    /// Updates the camera world-space AABB used to cull agent transform uploads.
    ///
    /// Call once per frame from GDScript with the camera's visible world rect,
    /// padded by ~200 m to avoid pop-in at the viewport edge. Agents outside the
    /// rect are excluded from the next `RenderSnapshot` transform buffers, reducing
    /// GPU upload cost from O(A_total) to O(A_visible).
    #[func]
    pub fn set_camera_aabb(&mut self, x_min: f32, x_max: f32, z_min: f32, z_max: f32) {
        let _ = self
            .cmd_tx
            .send(SimCommand::SetCameraAabb(x_min, x_max, z_min, z_max));
    }

    /// Maximum far-plane distance used when building the camera frustum AABB for agent culling.
    #[func]
    pub fn get_agent_cull_far_m() -> f32 {
        crate::config::AGENT_CULL_FAR_M
    }

    /// Padding added to each side of the camera frustum AABB to prevent pop-in.
    #[func]
    pub fn get_agent_cull_padding_m() -> f32 {
        crate::config::AGENT_CULL_PADDING_M
    }

    /// Target render FPS cap. Applied to `Engine.max_fps` at startup.
    #[func]
    pub fn get_target_fps() -> u32 {
        crate::config::TARGET_FPS
    }

    /// Returns the current simulation day count.
    #[func]
    pub fn get_current_day(&self) -> u32 {
        self.snapshot.read().unwrap().current_day
    }

    /// Returns the current operational minute since midnight.
    #[func]
    pub fn get_current_minute_of_day(&self) -> u16 {
        self.snapshot.read().unwrap().current_minute_of_day
    }

    /// Returns normalized residential, commercial, and industrial growth
    /// pressures for the gameplay HUD as `Vector3(x=residential, y=commercial, z=industrial)`.
    ///
    /// Each component is clamped to `-1.0..1.0`, corresponding to `-100%..100%`
    /// on the UI meter. This is a UI-facing read path only and is not used in
    /// the simulation tick.
    #[func]
    pub fn get_demand_pressures(&self) -> Vector3 {
        let core = self.lock_core();
        Vector3::new(
            core.demand.net_residential_pressure().clamp(-1.0, 1.0),
            core.demand.net_commercial_pressure().clamp(-1.0, 1.0),
            core.demand.net_industrial_pressure().clamp(-1.0, 1.0),
        )
    }

    // ── Agents ──

    /// Returns a Dictionary of packed transforms for visible non-car agents, keyed by pedestrian_type.
    #[func]
    pub fn get_agent_transforms(&self) -> VarDictionary {
        use super::sim::bridge::agents::get_agent_transforms;
        get_agent_transforms(&self.snapshot.read().unwrap())
    }

    /// Returns a Dictionary of packed transforms for visible car agents, keyed by vehicle type.
    #[func]
    pub fn get_car_transforms(&self) -> VarDictionary {
        use super::sim::bridge::agents::get_car_transforms;
        get_car_transforms(&self.snapshot.read().unwrap())
    }

    /// Returns debug path geometry for active agents.
    #[func]
    pub fn get_agent_paths_debug(&self) -> VarDictionary {
        self.lock_core().get_agent_paths_debug_internal()
    }

    // ── Buildings ──

    /// Returns `true` when the node was launched with `--asset-editor`.
    ///
    /// GDScript uses this to confirm it is running the editor shell rather than the normal
    /// city simulation, and to skip tick-driven systems that are not active in sandbox mode.
    #[func]
    pub fn is_asset_editor_mode(&self) -> bool {
        self.asset_editor_mode
    }

    /// Returns `true` when the node was launched with `--economy-editor`.
    #[func]
    pub fn is_economy_editor_mode(&self) -> bool {
        self.economy_editor_mode
    }

    /// Returns `true` when the node was launched with `--world-editor`.
    #[func]
    pub fn is_world_editor_mode(&self) -> bool {
        self.world_editor_mode
    }

    /// Loads the canonical authored economy folder and returns a JSON envelope
    /// containing profiles, controllers, scenarios, and validation messages.
    #[func]
    pub fn load_economy_project(&self, dir_path: GString) -> GString {
        use crate::simulation::economy::definitions::load_project_json;
        match load_project_json(std::path::Path::new(&dir_path.to_string())) {
            Ok(json) => GString::from(json.as_str()),
            Err(err) => {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err,
                    "validation": [],
                });
                GString::from(payload.to_string().as_str())
            }
        }
    }

    /// Validates the authored economy JSON payload, writes the canonical TOML
    /// files, and rebuilds the derived `economy.index.bin` cache.
    #[func]
    pub fn export_economy_project(&self, project_json: GString, dir_path: GString) -> GString {
        use crate::simulation::economy::definitions::export_project_json;
        match export_project_json(
            &project_json.to_string(),
            std::path::Path::new(&dir_path.to_string()),
        ) {
            Ok(json) => GString::from(json.as_str()),
            Err(err) => {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err,
                    "validation": [],
                });
                GString::from(payload.to_string().as_str())
            }
        }
    }

    /// Runs the small authored-economy sandbox for the selected scenario and
    /// returns daily series data plus summary bottleneck metrics as JSON.
    #[func]
    pub fn run_economy_sandbox(&self, project_json: GString, scenario_id: GString) -> GString {
        use crate::simulation::economy::definitions::run_sandbox_json;
        match run_sandbox_json(&project_json.to_string(), &scenario_id.to_string()) {
            Ok(json) => GString::from(json.as_str()),
            Err(err) => {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err,
                    "validation": [],
                });
                GString::from(payload.to_string().as_str())
            }
        }
    }

    /// Scans a native filesystem directory for content packs and registers all valid assets.
    #[func]
    pub fn load_asset_packs(&mut self, dir_path: GString, enabled_pack_ids: GString) -> GString {
        use super::sim::bridge::assets::load_asset_packs;
        load_asset_packs(&mut self.lock_core(), dir_path, enabled_pack_ids)
    }

    /// Returns all qualified asset IDs (`"pack_id:asset_id"`) currently in the registry.
    ///
    /// Godot uses this to enumerate which meshes to load for building rendering.
    #[func]
    pub fn get_registered_asset_ids(&self) -> PackedStringArray {
        let core = self.lock_core();
        let mut ids: Vec<GString> = core
            .allocator
            .registry
            .qualified_ids()
            .map(GString::from)
            .collect();

        let has_broken = core.allocator.buildings.iter().any(|b| b.broken);
        if has_broken {
            ids.push(GString::from("broken:error"));
        }

        PackedStringArray::from_iter(ids)
    }

    /// Returns the native filesystem path to the LOD0 mesh file for a registered asset.
    #[func]
    pub fn get_lod0_native_path(&self, qualified_id: GString) -> GString {
        use super::sim::bridge::assets::get_lod0_native_path;
        get_lod0_native_path(&self.lock_core(), qualified_id)
    }

    /// Returns the packed 12-float transforms for all placed buildings with the given asset ID.
    #[func]
    pub fn get_building_transforms_for_asset(&self, asset_id: GString) -> PackedFloat32Array {
        self.lock_core()
            .get_building_transforms_for_asset_internal(&asset_id.to_string())
    }

    /// Returns the packed 12-float transforms for all deserted buildings with the given asset ID.
    #[func]
    pub fn get_deserted_building_transforms_for_asset(
        &self,
        asset_id: GString,
    ) -> PackedFloat32Array {
        self.lock_core()
            .get_deserted_building_transforms_for_asset_internal(&asset_id.to_string())
    }

    /// Returns the packed transforms for building plots/foundations of a specific zone type.
    #[func]
    pub fn get_building_plot_transforms(&self, zone_type_int: u8) -> PackedFloat32Array {
        self.lock_core()
            .get_building_plot_transforms_internal(zone_type_int)
    }

    /// Returns a Dictionary of live stats for the building whose centre is closest to
    /// (`world_x`, `world_z`) within a 30 m pick radius.
    ///
    /// Returns an empty Dictionary when no building is within range.
    /// Keys: `asset_id`, `zone_type`, `level`, `occupancy`, `worker_count`,
    /// `worker_capacity`, `operating_budget`, `revenue`, `budget_distress`,
    /// `economy_broken`, `broken`, `pending_redevelopment`, `rezone_grace_days`,
    /// `economy_profile`, `center_x`, `center_z`, and `inventory`
    /// (Array of `{name, amount}` Dictionaries).
    #[func]
    pub fn get_building_info_at(&self, world_x: f32, world_z: f32) -> VarDictionary {
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;
        use crate::simulation::grid::zoning::ZoneType;

        let core = self.lock_core();

        // Linear scan — only called on explicit user clicks, never on the hot path.
        let pick_radius_sq = 30.0_f32 * 30.0;
        let mut best_idx = usize::MAX;
        let mut best_dist_sq = pick_radius_sq;
        for (i, b) in core.allocator.buildings.iter().enumerate() {
            let dx = b.center_x - world_x;
            let dz = b.center_y - world_z; // center_y is world-Z in the building struct
            let dist_sq = dx * dx + dz * dz;
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best_idx = i;
            }
        }
        if best_idx == usize::MAX {
            return VarDictionary::new();
        }

        let b = &core.allocator.buildings[best_idx];
        let catalog = load_runtime_economy_catalog().ok();

        let zone_type_str = match b.zone_type {
            ZoneType::None => "utility",
            ZoneType::Residential => "residential",
            ZoneType::Commercial => "commercial",
            ZoneType::Industrial => "industrial",
            ZoneType::Office => "office",
            ZoneType::Mixed => "mixed",
        };

        let profile_id = catalog
            .as_ref()
            .and_then(|c| c.profile_by_runtime_id(b.economy_profile_runtime_id))
            .map(|p| p.id.clone())
            .unwrap_or_default();

        let worker_capacity = core.allocator.worker_capacity(best_idx);

        // Inventory: only non-zero resource slots.
        let mut inv_arr = VarArray::new();
        if let Some(cat) = &catalog {
            for (slot, &amount) in b.resource_inventory.iter().enumerate() {
                if amount > 0.001 {
                    let runtime_id = (slot + 1) as u16;
                    let name = cat
                        .resource_id_for_runtime_id(runtime_id)
                        .unwrap_or("unknown");
                    let mut entry = VarDictionary::new();
                    entry.set("name", GString::from(name));
                    entry.set("amount", amount as f64);
                    inv_arr.push(&entry.to_variant());
                }
            }
        }

        let mut dict = VarDictionary::new();
        dict.set("asset_id", GString::from(b.asset_id.as_str()));
        dict.set("zone_type", GString::from(zone_type_str));
        dict.set("level", b.level as i32);
        dict.set("occupancy", b.occupancy as i32);
        dict.set("center_x", b.center_x as f64);
        dict.set("center_z", b.center_y as f64);

        let mut total_agents = 0i32;
        if b.zone_type == ZoneType::Residential {
            for h in &core.households.households {
                if h.home_building_id == best_idx && h.member_count > 0 {
                    total_agents += h.member_count as i32;
                }
            }
        }
        dict.set("agent_count", total_agents);
        dict.set("worker_count", b.worker_count as i32);
        dict.set("worker_capacity", worker_capacity as i32);
        dict.set("operating_budget", b.operating_budget as f64);
        dict.set("revenue", b.revenue as f64);
        dict.set("budget_distress", b.budget_distress);
        dict.set("economy_broken", b.economy_broken);
        dict.set("broken", b.broken);
        dict.set("pending_redevelopment", b.pending_redevelopment);
        dict.set("rezone_grace_days", b.rezone_grace_days_remaining as i32);
        dict.set("economy_profile", GString::from(profile_id.as_str()));
        dict.set("inventory", inv_arr.to_variant());
        dict
    }

    /// Validates the JSON export params, writes `pack.toml` (if absent) and
    /// `assets/<asset_id>/asset.toml` under `output_dir`, and returns an error
    /// string or `""` on success.
    ///
    /// `output_dir` must be an absolute native path (resolve `user://mods/<pack_id>/`
    /// with `ProjectSettings.globalize_path` before passing it in).
    #[func]
    pub fn validate_and_export_asset(&self, params_json: GString, output_dir: GString) -> GString {
        use crate::nodes::sim::asset_export::validate_and_export_asset_internal;
        let result =
            validate_and_export_asset_internal(&params_json.to_string(), &output_dir.to_string());
        GString::from(result.as_str())
    }

    /// Returns a JSON object describing the manifest for an already-registered asset,
    /// or `""` if the qualified ID is not in the registry.
    ///
    /// GDScript uses this to repopulate the importer form when re-editing an existing asset.
    #[func]
    pub fn get_asset_manifest_json(&self, qualified_id: GString) -> GString {
        use crate::nodes::sim::asset_export::get_asset_manifest_json_internal;
        let core = self.lock_core();
        let result =
            get_asset_manifest_json_internal(&core.allocator.registry, &qualified_id.to_string());
        GString::from(result.as_str())
    }

    /// Returns a JSON object with pack metadata (`pack_id`, `display_name`, `author`,
    /// `version`, `license`) read from `<output_dir>/pack.toml`, or `""` if not found.
    ///
    /// `output_dir` must be the absolute native path to the pack directory
    /// (i.e. `ProjectSettings.globalize_path("user://mods/<pack_id>/")` ).
    #[func]
    pub fn get_pack_manifest_json(&self, output_dir: GString) -> GString {
        use crate::nodes::sim::asset_export::get_pack_manifest_json_internal;
        GString::from(get_pack_manifest_json_internal(&output_dir.to_string()).as_str())
    }

    // ── Network ──

    /// Returns the closest boundary point on a road edge to the given position.
    #[func]
    pub fn get_closest_point_on_edge(&self, edge_idx: i32, point_x: f32, point_y: f32) -> Vector2 {
        self.lock_core()
            .get_closest_point_on_edge_internal(edge_idx, point_x, point_y)
    }

    /// Returns the physical segment geometry for a road edge.
    #[func]
    pub fn get_edge_geometry(&self, edge_idx: i32) -> PackedVector2Array {
        self.lock_core().get_edge_geometry_internal(edge_idx)
    }

    /// Returns the 3D geometry for a road edge.
    #[func]
    pub fn get_edge_geometry_3d(&self, edge_idx: i32) -> PackedVector3Array {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return PackedVector3Array::new();
        }
        let edge = core.region_graph.edge(edge_idx as usize);
        PackedVector3Array::from_iter(edge.physical_geometry.iter().cloned())
    }

    /// Returns the width of a specific road edge.
    #[func]
    pub fn get_edge_width(&self, edge_idx: i32) -> f32 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return 6.0;
        }
        core.region_graph.edge(edge_idx as usize).width
    }

    /// Returns a curved frontage between two points on an edge.
    #[func]
    pub fn get_curved_frontage(
        &self,
        edge_idx: i32,
        start_p: Vector2,
        end_p: Vector2,
    ) -> PackedVector2Array {
        self.lock_core()
            .get_curved_frontage_internal(edge_idx, start_p, end_p)
    }

    /// Adds a new road segment to the network.
    #[func]
    pub fn add_road(&mut self, points: PackedVector3Array, fwd_lanes: i32, bkw_lanes: i32) {
        // Send to the background thread so the Godot main thread is never blocked
        // by the expensive lane-rebuild and zoning-obstruction passes (~500 ms).
        // The road appears on the next sim tick (~16 ms later) — imperceptible delay.
        let _ = self
            .cmd_tx
            .send(crate::nodes::sim::core::SimCommand::AddRoad {
                points: points.to_vec(),
                fwd_lanes,
                bkw_lanes,
            });
    }

    /// Returns the node ID of the nearest graph node near the border, or -1.
    #[func]
    pub fn check_border_candidate(&self, pos: Vector3) -> i64 {
        self.lock_core().check_border_candidate_internal(pos)
    }

    /// Marks the node at `node_id` as an external border connection.
    #[func]
    pub fn set_border_connection(&mut self, node_id: i32) {
        self.lock_core().set_border_connection_internal(node_id);
    }

    /// Returns the world-space positions of all active border nodes as a flat float array.
    #[func]
    pub fn get_border_nodes(&self) -> PackedFloat32Array {
        self.lock_core().get_border_nodes_internal()
    }

    /// Returns the classification of an edge as an integer (0=Standard, 1=Bridge, 2=Tunnel).
    /// Returns 0 if the edge index is invalid.
    #[func]
    pub fn get_edge_class(&self, edge_idx: i32) -> u8 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return 0;
        }
        match core.region_graph.edge(edge_idx as usize).class {
            crate::simulation::network::types::EdgeClass::Bridge => 1,
            crate::simulation::network::types::EdgeClass::Tunnel => 2,
            _ => 0,
        }
    }

    /// Sets the classification of an edge (Standard, Bridge, Tunnel).
    #[func]
    pub fn set_edge_class(&mut self, edge_idx: i32, class_int: u8) {
        self.lock_core()
            .set_edge_class_internal(edge_idx, class_int);
    }

    /// Sets or clears the no-building-spawn flag on an edge. When true the building
    /// allocator skips this edge. Player-toggleable; also auto-set for speed ≥ 80 km/h.
    #[func]
    pub fn set_no_building_spawn(&mut self, edge_idx: i32, enabled: bool) {
        self.lock_core()
            .set_no_building_spawn_internal(edge_idx, enabled);
    }

    /// Sets the vehicle frontage-access policy on an edge.
    ///
    /// `0 = SameSideOnly`, `1 = BothSides`. Invalid values are ignored.
    #[func]
    pub fn set_vehicle_frontage_access(&mut self, edge_idx: i32, access_int: u8) {
        self.lock_core()
            .set_vehicle_frontage_access_internal(edge_idx, access_int);
    }

    /// Returns true if the given edge has the no-building-spawn flag set.
    #[func]
    pub fn get_no_building_spawn(&self, edge_idx: i32) -> bool {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return false;
        }
        core.region_graph.edge(edge_idx as usize).no_building_spawn
    }

    /// Returns the vehicle frontage-access policy on an edge.
    ///
    /// Returns `1` (`BothSides`) if the edge index is invalid.
    #[func]
    pub fn get_vehicle_frontage_access(&self, edge_idx: i32) -> u8 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return 1;
        }
        match core
            .region_graph
            .edge(edge_idx as usize)
            .vehicle_frontage_access
        {
            crate::simulation::network::types::VehicleFrontageAccess::SameSideOnly => 0,
            crate::simulation::network::types::VehicleFrontageAccess::BothSides => 1,
        }
    }

    /// Returns the start and end node indices of an edge as `Vector2i(start, end)`.
    /// Returns `(-1, -1)` if the edge index is invalid.
    #[func]
    pub fn get_edge_nodes(&self, edge_idx: i32) -> Vector2i {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return Vector2i::new(-1, -1);
        }
        let e = core.region_graph.edge(edge_idx as usize);
        Vector2i::new(e.start_node as i32, e.end_node as i32)
    }

    /// Returns the indices of all non-deleted edges with `no_building_spawn = true`.
    /// Used by the zone-tool overlay to draw the hatched no-build indicator.
    #[func]
    pub fn get_no_building_spawn_edge_indices(&self) -> PackedInt32Array {
        let core = self.lock_core();
        let mut out = PackedInt32Array::new();
        for (i, e) in core.region_graph.edges().iter().enumerate() {
            if !e.deleted && e.no_building_spawn {
                out.push(i as i32);
            }
        }
        out
    }

    /// Returns dictionary of road/intersection mesh data.
    #[func]
    pub fn get_road_mesh_data(&self) -> VarDictionary {
        let mut core = self.lock_core();
        core.get_road_mesh_data_internal()
    }

    /// Returns temporary compiled preview-surface data for the road tool.
    ///
    /// Uses `try_lock` because this is called during live mouse movement and must not stall the
    /// Godot main thread while the sim thread is holding the core mutex.
    #[func]
    pub fn get_preview_road_surface(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> Variant {
        let fwd_lanes = fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let bkw_lanes = bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        match self.try_lock_core() {
            Some(core) => core
                .get_preview_road_surface_internal(points, fwd_lanes, bkw_lanes)
                .to_variant(),
            None => Variant::nil(),
        }
    }

    /// Returns compiled road-surface debug line data for editor visualization.
    ///
    /// Uses `try_lock` because this is only a debug/editor helper and should never stall the
    /// Godot main thread while the simulation mutex is busy.
    #[func]
    pub fn get_road_surface_debug_data(&self) -> Variant {
        match self.try_lock_core() {
            Some(mut core) => core.get_road_surface_debug_data_internal().to_variant(),
            None => Variant::nil(),
        }
    }

    /// Returns terrain render patches that must keep full mesh resolution where compiled road ownership is visible.
    #[func]
    pub fn get_road_locked_terrain_patches(&self) -> PackedInt32Array {
        let mut core = self.lock_core();
        core.get_road_locked_terrain_patches_internal()
    }

    /// Returns the closest network point (node/edge) within range.
    ///
    /// Uses `try_lock` for the same reason as `intersect_terrain` — called every
    /// frame while the road tool is active; must not stall on the sim mutex.
    /// Returns `null` when contended; GDScript handles null from this call already.
    #[func]
    pub fn get_closest_network_point(&self, world_pos: Vector3, max_dist: f32) -> Variant {
        match self.try_lock_core() {
            Some(core) => match core.get_closest_network_point_internal(world_pos, max_dist) {
                Some(p) => p.to_variant(),
                None => Variant::nil(),
            },
            None => Variant::nil(),
        }
    }

    /// Returns the ID of the closest network node.
    #[func]
    pub fn get_closest_node(&self, world_pos: Vector3, max_dist: f32) -> i32 {
        self.lock_core()
            .get_closest_node_internal(world_pos, max_dist)
    }

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn set_node_cul_de_sac(&mut self, _node_id: i32, _enabled: bool, _radius: f32) {}

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn has_cul_de_sac(&self, _node_id: i32) -> bool {
        false
    }

    /// Returns the number of road connections for a node.
    #[func]
    pub fn get_node_connection_count(&self, node_id: i32) -> i32 {
        self.lock_core().get_node_connection_count_internal(node_id)
    }

    /// Repositions a network node.
    #[func]
    pub fn move_network_node(&mut self, node_id: i32, pos: Vector3) {
        self.lock_core().move_network_node_internal(node_id, pos);
    }

    /// Returns all junction node positions, read from the pre-computed snapshot.
    ///
    /// Reading from the snapshot (RwLock) avoids acquiring the SimCore mutex, which
    /// would stall the Godot main thread while `add_road_internal` holds the lock.
    #[func]
    pub fn get_network_nodes(&self) -> PackedVector3Array {
        PackedVector3Array::from_iter(self.snapshot.read().unwrap().node_positions.iter().copied())
    }

    /// Returns ghost guide data for the road-tool overlay.
    #[func]
    pub fn get_road_ghost_guides(&self) -> PackedFloat32Array {
        use super::sim::bridge::network::get_road_ghost_guides;
        match self.try_lock_core() {
            Some(core) => get_road_ghost_guides(&core),
            None => PackedFloat32Array::new(),
        }
    }

    /// Returns the full physical geometry of every non-deleted road edge.
    #[func]
    pub fn get_road_edge_polylines(&self) -> PackedFloat32Array {
        use super::sim::bridge::network::get_road_edge_polylines;
        match self.try_lock_core() {
            Some(core) => get_road_edge_polylines(&core),
            None => PackedFloat32Array::new(),
        }
    }

    /// Returns the tangent direction of the road whose endpoint is nearest to `pos`
    /// within `max_dist` metres.
    ///
    /// Returns the road tangent direction closest to `pos` within `max_dist` metres.
    ///
    /// Walks the physical geometry of every non-deleted edge and finds the closest point
    /// on the polyline (not just endpoints), returning the segment tangent at that point.
    /// This ensures mid-road snaps give the correct perpendicular direction.
    ///
    /// The returned `Vector2` is `(world_x, world_z)` of the unit tangent (either
    /// direction along the edge — callers only need the axis, not the sign).
    /// Returns `Vector2(0, 1)` (world +Z) if no road is within range.
    ///
    /// Non-blocking: returns `Vector2(0, 1)` if the SimCore mutex is contended.
    /// Returns the road tangent direction closest to `pos` within `max_dist` metres.
    #[func]
    pub fn get_road_tangent_at(&self, pos: Vector3, max_dist: f32) -> Vector2 {
        use super::sim::bridge::network::get_road_tangent_at;
        match self.try_lock_core() {
            Some(core) => get_road_tangent_at(&core, pos, max_dist),
            None => Vector2::new(0.0, 1.0),
        }
    }

    /// Configures a lane connection rule at a junction.
    #[func]
    pub fn set_lane_connection(
        &mut self,
        node_id: u32,
        from_edge: i32,
        from_lane: i32,
        to_edge: i32,
        to_lane: i32,
    ) {
        self.lock_core()
            .set_lane_connection_internal(node_id, from_edge, from_lane, to_edge, to_lane);
    }

    /// Clears all lane rules at a junction node.
    #[func]
    pub fn clear_lane_connections(&mut self, node_id: u32) {
        self.lock_core().clear_lane_connections_internal(node_id);
    }

    /// Toggles a user override for a crosswalk at a specific road mouth.
    #[func]
    pub fn set_crosswalk_override(&mut self, node_id: u32, edge_id: i32, enabled: bool) {
        self.lock_core()
            .set_crosswalk_override_internal(node_id, edge_id, enabled);
    }

    /// Returns true if a crosswalk exists natively or by user override.
    #[func]
    pub fn has_crosswalk(&self, node_id: u32, edge_id: i32) -> bool {
        self.lock_core().has_crosswalk_internal(node_id, edge_id)
    }

    /// Returns the world-space position of a node.
    #[func]
    pub fn get_node_pos(&self, node_id: u32) -> Vector3 {
        self.lock_core().get_node_pos_internal(node_id)
    }

    /// Returns information about all lanes entering/leaving a junction.
    #[func]
    pub fn get_node_lanes(&self, node_id: u32) -> VarArray {
        self.lock_core().get_node_lanes_internal(node_id)
    }

    /// Returns an array of current lane turn restrictions at a node.
    #[func]
    pub fn get_lane_connections_array(&self, node_id: u32) -> VarArray {
        self.lock_core()
            .get_lane_connections_array_internal(node_id)
    }

    /// Clears lane rules for a specific source lane.
    #[func]
    pub fn clear_lane_source(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        self.lock_core()
            .clear_lane_source_internal(node_id, from_edge, from_lane);
    }

    /// Returns the average network direction at a given point.
    #[func]
    pub fn get_network_direction_at_point(&self, pos: Vector3) -> Vector3 {
        self.lock_core()
            .get_network_direction_at_point_internal(pos)
    }

    /// Rebuilds visual terrain from the authoritative road-surface cache.
    #[func]
    pub fn rebuild_network_surface_terrain(&mut self) {
        self.lock_core().rebuild_network_surface_terrain_internal();
    }

    /// Returns terrain height at a position.
    #[func]
    pub fn get_height_at(&self, pos: Vector2) -> f32 {
        self.lock_core().get_height_at_internal(pos)
    }

    /// Returns the visible world-surface height at a position.
    ///
    /// This prefers the compiled roadbed when a road surface owns the queried XZ location and
    /// otherwise falls back to the current visual terrain.
    #[func]
    pub fn get_world_surface_height(&self, pos: Vector2) -> f32 {
        self.lock_core().get_world_surface_height_internal(pos)
    }

    /// Raycasts against the terrain heightmap.
    ///
    /// Uses `try_lock` so this never stalls the Godot main thread if the sim thread
    /// is currently holding the mutex (e.g. during `add_road_internal`). Returns
    /// `null` when contended; GDScript already handles null from this call gracefully.
    #[func]
    pub fn intersect_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Variant {
        match self.try_lock_core() {
            Some(core) => match core.intersect_terrain_internal(ray_origin, ray_dir) {
                Some(p) => p.to_variant(),
                None => Variant::nil(),
            },
            None => Variant::nil(),
        }
    }

    /// Raycasts against the visible world surface.
    ///
    /// Uses `try_lock` for the same reason as `intersect_terrain`. The combined surface prefers
    /// compiled roadbed ownership and otherwise falls back to the visible terrain surface.
    #[func]
    pub fn intersect_world_surface(&self, ray_origin: Vector3, ray_dir: Vector3) -> Variant {
        match self.try_lock_core() {
            Some(mut core) => match core.intersect_world_surface_internal(ray_origin, ray_dir) {
                Some(p) => p.to_variant(),
                None => Variant::nil(),
            },
            None => Variant::nil(),
        }
    }

    /// Resets the runtime to a new blank authored world with the given terrain settings.
    #[func]
    pub fn create_blank_world(
        &mut self,
        width_m: f32,
        height_m: f32,
        terrain_cell_m: f32,
        terrain_chunk_m: f32,
        base_elevation_m: f32,
    ) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.create_blank_world_internal(
                width_m,
                height_m,
                terrain_cell_m,
                terrain_chunk_m,
                base_elevation_m,
            )
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Create blank world failed: {}", err);
                false
            }
        }
    }

    /// Saves the current simulation into a single SQLite snapshot file.
    #[func]
    pub fn save_game(&self, path: GString) -> bool {
        match self.lock_core().save_game_internal(&path.to_string()) {
            Ok(()) => true,
            Err(err) => {
                godot_error!("Save failed: {}", err);
                false
            }
        }
    }

    /// Saves the current blank-world authoring state as a reusable world-definition asset.
    #[func]
    pub fn save_world_definition(&self, path: GString, name: GString) -> bool {
        match self
            .lock_core()
            .save_world_definition_internal(&path.to_string(), &name.to_string())
        {
            Ok(()) => true,
            Err(err) => {
                godot_error!("Save world definition failed: {}", err);
                false
            }
        }
    }

    /// Loads a SQLite save snapshot and replaces the live simulation state.
    #[func]
    pub fn load_game(&mut self, path: GString) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.load_game_internal(&path.to_string())
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Load failed: {}", err);
                false
            }
        }
    }

    /// Loads a reusable world-definition asset and replaces the live runtime with a blank city.
    #[func]
    pub fn load_world_definition(&mut self, path: GString) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.load_world_definition_internal(&path.to_string())
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Load world definition failed: {}", err);
                false
            }
        }
    }

    /// Adds or strengthens one authored world-water source at the clicked terrain cell.
    #[func]
    pub fn add_world_water_source(&mut self, pos: Vector2, rate_m_per_tick: f32) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.add_world_water_source_internal(pos, rate_m_per_tick)
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Add world water source failed: {}", err);
                false
            }
        }
    }

    /// Adds or strengthens one authored world-water sink at the clicked terrain cell.
    #[func]
    pub fn add_world_water_sink(&mut self, pos: Vector2, rate_m_per_tick: f32) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.add_world_water_sink_internal(pos, rate_m_per_tick)
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Add world water sink failed: {}", err);
                false
            }
        }
    }

    /// Removes the nearest authored world-water source within the given radius.
    #[func]
    pub fn remove_world_water_source_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_water_source_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Removes the nearest authored world-water sink within the given radius.
    #[func]
    pub fn remove_world_water_sink_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_water_sink_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Starts one transient authored lake-fill preview at the clicked terrain cell.
    #[func]
    pub fn begin_world_lake_fill_preview(
        &mut self,
        pos: Vector2,
        surface_elevation_m: f32,
    ) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.begin_world_lake_fill_preview_internal(pos, surface_elevation_m)
        };
        self.refresh_snapshot_from_core();
        match result {
            Ok(preview) => {
                Self::world_lake_fill_preview_dict(Some(preview), true, "lake fill preview updated")
            }
            Err(err) => {
                godot_error!("Begin world lake fill preview failed: {}", err);
                Self::world_lake_fill_preview_dict(None, false, &err)
            }
        }
    }

    /// Starts one transient authored open-water preview at the clicked terrain cell.
    #[func]
    pub fn begin_world_open_water_fill_preview(
        &mut self,
        pos: Vector2,
        surface_elevation_m: f32,
    ) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.begin_world_open_water_fill_preview_internal(pos, surface_elevation_m)
        };
        self.refresh_snapshot_from_core();
        match result {
            Ok(preview) => Self::world_lake_fill_preview_dict(
                Some(preview),
                true,
                "open water preview updated",
            ),
            Err(err) => {
                godot_error!("Begin world open water preview failed: {}", err);
                Self::world_lake_fill_preview_dict(None, false, &err)
            }
        }
    }

    /// Updates the active transient lake-fill preview surface elevation.
    #[func]
    pub fn update_world_lake_fill_preview(&mut self, surface_elevation_m: f32) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.update_world_lake_fill_preview_internal(surface_elevation_m)
        };
        match result {
            Ok(preview) => {
                self.refresh_snapshot_from_core();
                Self::world_lake_fill_preview_dict(Some(preview), true, "lake fill preview updated")
            }
            Err(err) => {
                godot_error!("Update world lake fill preview failed: {}", err);
                let active_preview = self.lock_core().world_water_fill_preview_internal();
                Self::world_lake_fill_preview_dict(active_preview, false, &err)
            }
        }
    }

    /// Updates the active transient open-water preview surface elevation.
    #[func]
    pub fn update_world_open_water_fill_preview(
        &mut self,
        surface_elevation_m: f32,
    ) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.update_world_open_water_fill_preview_internal(surface_elevation_m)
        };
        match result {
            Ok(preview) => {
                self.refresh_snapshot_from_core();
                Self::world_lake_fill_preview_dict(
                    Some(preview),
                    true,
                    "open water preview updated",
                )
            }
            Err(err) => {
                godot_error!("Update world open water preview failed: {}", err);
                let active_preview = self.lock_core().world_water_fill_preview_internal();
                Self::world_lake_fill_preview_dict(active_preview, false, &err)
            }
        }
    }

    /// Returns the current transient lake-fill preview state.
    #[func]
    pub fn get_world_lake_fill_preview(&self) -> VarDictionary {
        let preview = self.lock_core().world_water_fill_preview_internal();
        let message = if preview.is_some() {
            "surface fill preview active"
        } else {
            "no surface fill preview is active"
        };
        Self::world_lake_fill_preview_dict(preview, true, message)
    }

    /// Returns the current transient open-water preview state.
    #[func]
    pub fn get_world_open_water_fill_preview(&self) -> VarDictionary {
        let preview = self.lock_core().world_water_fill_preview_internal();
        let message = if preview.is_some() {
            "open water preview active"
        } else {
            "no open water preview is active"
        };
        Self::world_lake_fill_preview_dict(preview, true, message)
    }

    /// Returns committed authored-world water markers for world-editor overlays.
    #[func]
    pub fn get_world_water_authoring_markers(&self) -> VarArray {
        let core = self.lock_core();
        let mut markers = VarArray::new();

        for point in &core.world_water_boundary_points {
            let terrain_height_m = core
                .heightmap
                .sample_height_world(point.world_x, point.world_z)
                * config::HEIGHT_SCALE;
            let kind = match point.kind {
                crate::simulation::world_definition::AuthoredWaterBoundaryKind::Source => "source",
                crate::simulation::world_definition::AuthoredWaterBoundaryKind::Sink => "sink",
            };
            markers.push(
                &Self::world_water_authoring_marker_dict(
                    kind,
                    point.world_x,
                    point.world_z,
                    terrain_height_m,
                    None,
                    Some(point.rate_m_per_tick),
                )
                .to_variant(),
            );
        }

        for lake in &core.world_lake_fills {
            let terrain_height_m = core
                .heightmap
                .sample_height_world(lake.world_x, lake.world_z)
                * config::HEIGHT_SCALE;
            markers.push(
                &Self::world_water_authoring_marker_dict(
                    "lake_fill",
                    lake.world_x,
                    lake.world_z,
                    terrain_height_m,
                    Some(lake.surface_elevation_m),
                    None,
                )
                .to_variant(),
            );
        }

        for open_water in &core.world_open_water_fills {
            let terrain_height_m = core
                .heightmap
                .sample_height_world(open_water.world_x, open_water.world_z)
                * config::HEIGHT_SCALE;
            markers.push(
                &Self::world_water_authoring_marker_dict(
                    "open_water_fill",
                    open_water.world_x,
                    open_water.world_z,
                    terrain_height_m,
                    Some(open_water.surface_elevation_m),
                    None,
                )
                .to_variant(),
            );
        }

        markers
    }

    /// Commits the active transient lake-fill preview into authored world state.
    #[func]
    pub fn commit_world_lake_fill_preview(&mut self) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.commit_world_lake_fill_preview_internal()
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Commit world lake fill preview failed: {}", err);
                false
            }
        }
    }

    /// Commits the active transient open-water preview into authored world state.
    #[func]
    pub fn commit_world_open_water_fill_preview(&mut self) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.commit_world_open_water_fill_preview_internal()
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Commit world open water preview failed: {}", err);
                false
            }
        }
    }

    /// Cancels the active transient lake-fill preview.
    #[func]
    pub fn cancel_world_lake_fill_preview(&mut self) -> bool {
        let cancelled = {
            let mut core = self.lock_core();
            core.cancel_world_water_fill_preview_internal()
        };
        if cancelled {
            self.refresh_snapshot_from_core();
        }
        cancelled
    }

    /// Cancels the active transient open-water preview.
    #[func]
    pub fn cancel_world_open_water_fill_preview(&mut self) -> bool {
        let cancelled = {
            let mut core = self.lock_core();
            core.cancel_world_water_fill_preview_internal()
        };
        if cancelled {
            self.refresh_snapshot_from_core();
        }
        cancelled
    }

    /// Removes the nearest authored lake fill within the given radius.
    #[func]
    pub fn remove_world_lake_fill_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_lake_fill_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Removes the nearest authored open-water fill within the given radius.
    #[func]
    pub fn remove_world_open_water_fill_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_open_water_fill_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Returns the current city treasury balance in currency units. May be negative.
    #[func]
    pub fn get_treasury_balance(&self) -> f64 {
        self.snapshot.read().unwrap().treasury_balance
    }

    /// Returns the total number of live agents from the latest render snapshot.
    #[func]
    pub fn get_agent_count(&self) -> i32 {
        self.snapshot.read().unwrap().agent_count
    }

    /// Returns global lane width.
    #[func]
    pub fn get_lane_width(&self) -> f32 {
        config::LANE_WIDTH
    }

    /// High-level city setup for performance testing.
    #[func]
    pub fn setup_benchmark_city(&mut self, grid_size: i32, agent_count: i32) {
        self.lock_core()
            .setup_benchmark_city_internal(grid_size, agent_count);
    }

    /// Returns performance stats (ms, FPS, agents).
    #[func]
    pub fn get_perf_stats(&self) -> VarDictionary {
        self.get_perf_stats_internal()
    }

    /// Returns zone grid dimensions for texture setup.
    #[func]
    pub fn get_zone_grid_size(&self) -> Vector2i {
        let core = self.lock_core();
        Vector2i::new(
            core.zoning.grid.width as i32,
            core.zoning.grid.height as i32,
        )
    }
}

#[godot_api]
impl INode3D for SimulationNode {
    fn init(base: Base<Node3D>) -> Self {
        debug_log!("init", "Simulation Engine Initialized");

        let args = godot::classes::Os::singleton().get_cmdline_user_args();
        let mut generate_benchmark = false;
        let mut run_benchmark = false;
        let mut asset_editor_mode = false;
        let mut economy_editor_mode = false;
        let mut world_editor_mode = false;
        for arg in args.as_slice() {
            match arg.to_string().as_str() {
                "--huge-map" | "--benchmark" => {
                    run_benchmark = true;
                }
                "--generate-benchmark" => {
                    generate_benchmark = true;
                }
                "--asset-editor" => {
                    asset_editor_mode = true;
                    // Always enable debug logging in the asset editor so creators
                    // can follow export/validation output in the terminal.
                    crate::debug::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                "--economy-editor" => {
                    economy_editor_mode = true;
                    crate::debug::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                "--world-editor" => {
                    world_editor_mode = true;
                }
                _ => {}
            }
        }

        let config = if asset_editor_mode || economy_editor_mode || world_editor_mode {
            WorldConfig::editor_sandbox()
        } else {
            WorldConfig::gameplay_default()
        };

        let benchmark_mode = run_benchmark || generate_benchmark;

        let core = SimCore {
            time: TimeSystem::new(),
            heightmap: TerrainSystem::from_world_config(&config),
            watermap: WaterSystem::from_world_config(&config),
            region_graph: crate::simulation::network::graph::RegionGraph::new(),
            transit_network: TransitNetwork::new_with_surface_chunk_span(config.terrain_chunk_m),
            zoning: ZoningSystem::new(&config),
            pollution: PollutionSystem::new(&config),
            noise: NoiseSystem::new(&config),
            desirability: DesirabilitySystem::new(&config),
            demand: DemandSystem::new(),
            allocator: BuildingAllocator::new(),
            agents: AgentSystem::new(),
            households: HouseholdSystem::new(),
            logistics: ShipmentSystem::new(),
            config,
            treasury: CityTreasury::new(
                load_runtime_economy_tuning()
                    .map(|t| t.startup_treasury_balance)
                    .unwrap_or(100_000.0),
            ),
            undo_stack: VecDeque::new(),
            world_water_boundary_points: Vec::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
            world_lake_fill_preview: None,
            terrain_stroke_active: false,
            terrain_stroke_has_changes: false,
            water_runtime_realtime_when_paused: world_editor_mode,
            terrain_dirty: true,
            water_dirty: true,
            network_dirty: false,
            benchmark_mode,
            last_tick_duration: 0.0,
            last_agent_tick_us: 0,
            last_road_timing: String::new(),
            last_surface_debug_edges: Vec::new(),
            camera_aabb: (0.0, 0.0, 0.0, 0.0), // 0.0 == 0.0 → cull disabled by default
        };

        let core_arc = Arc::new(Mutex::new(core));
        let snapshot = Arc::new(RwLock::new(RenderSnapshot::default()));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();

        if generate_benchmark {
            godot_print!("BENCHMARK GENERATION MODE — will build city, save, and exit");
        } else if run_benchmark {
            godot_print!("BENCHMARK RUN MODE — will load benchmark.sav and simulate");
        }

        Self {
            core: core_arc,
            snapshot,
            sim_thread: None,
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            benchmark_mode,
            asset_editor_mode,
            economy_editor_mode,
            world_editor_mode,
            benchmark_tick_count: 0,
            last_logged_day: 0,
            time_passed: 0.0,
            base,
        }
    }

    fn ready(&mut self) {
        godot::classes::Engine::singleton().set_max_fps(crate::config::TARGET_FPS as i32);

        let args = godot::classes::Os::singleton().get_cmdline_user_args();
        let generate = args
            .as_slice()
            .iter()
            .any(|a| a.to_string() == "--generate-benchmark");
        let run = args
            .as_slice()
            .iter()
            .any(|a| matches!(a.to_string().as_str(), "--benchmark" | "--huge-map"));

        if generate {
            self.generate_benchmark_map();
            return; // generate_benchmark_map() calls quit() — never reaches thread spawn
        } else if run {
            self.run_benchmark_from_save();
        }

        // Asset editor mode: sandbox only — no simulation thread.
        if self.asset_editor_mode {
            godot_print!(
                "[asset-editor] sandbox ready — {:.0} m world, no simulation thread",
                WorldConfig::EDITOR_SANDBOX_WIDTH_M
            );
            debug_log!("asset-editor", "sandbox ready");
            return;
        }
        if self.economy_editor_mode {
            godot_print!(
                "[economy-editor] shell ready — authoritative economy data only, no simulation thread"
            );
            debug_log!("economy-editor", "shell ready");
            return;
        }
        if self.world_editor_mode {
            godot_print!(
                "[world-editor] shell ready — blank-world authoring runtime, simulation thread available"
            );
            debug_log!("world-editor", "shell ready");
        }

        // Start the background simulation thread.
        self.start_sim_thread();
    }

    fn process(&mut self, delta: f64) {
        self.time_passed += delta;

        if self.benchmark_mode {
            self.benchmark_tick_count += 1;

            // Periodic console log every 600 frames.
            if self.benchmark_tick_count % 600 == 0 {
                let snap = self.snapshot.read().unwrap();
                godot_print!(
                    "[bench] frame={} agents={} agent_tick_us={} sim_tick_ms={:.2} pathfinds={} RSS={}MB",
                    self.benchmark_tick_count,
                    snap.agent_count,
                    snap.last_agent_tick_us,
                    snap.last_tick_ms,
                    snap.pathfind_count,
                    crate::nodes::sim::benchmark::rss_mb()
                );
            }

            // CSV log once per in-game day.
            {
                let day = self.snapshot.read().unwrap().current_day;
                if day > self.last_logged_day {
                    self.last_logged_day = day;
                    self.log_benchmark_to_csv();
                }
            }

            if self.benchmark_tick_count >= 3000 {
                godot_print!("[bench] DONE — 3000 frames complete. See benchmark_results.csv.");
                self.base_mut().get_tree().unwrap().quit();
            }
        }
    }
}
