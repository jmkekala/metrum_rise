// SPDX-License-Identifier: GPL-2.0-only

//! Headless growth scenario for economy balancing (`ECON-12`).
//!
//! Builds a small zoned town through the player's own tools (road commits, border connection,
//! cell paint) with the repository's bootstrap asset pack, then runs it with
//! [`SimCore::step_frame`] at maximum speed. Agents move exactly as in the game, because a
//! household is admitted only once its carrier reaches home.
//!
//! Bootstrap meshes are not imported here (that needs Godot), so building sites use the
//! meshless support path. Results are deterministic for one build and asset pack, not across
//! changes to either; see `docs/economy.md#econ-12-growth-scenario`.

use std::path::Path;
use std::time::Instant;

use glam::DVec2;
use godot::prelude::Vector3;

use super::frame::FRAME_DT_S;
use super::road_commit::RoadCommitRequest;
use super::state::SimCore;
use crate::assets::scan_pack_dir;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::SIMULATION_SPEED_STEPS;
use crate::simulation::zoning::ZoneType;
use crate::simulation::zoning::cells::CellSelectionShape;
use crate::simulation::zoning::profiles::ZoneDensity;

const WORLD_SIZE_M: f32 = 2048.0;
// The trunk starts this far inside the terrain edge, within `BORDER_DETECTION_THRESHOLD`.
const BORDER_INSET_M: f32 = 1.0;
const MAIN_STREET_END_X_M: f32 = 300.0;
const CROSS_STREET_X_M: [f32; 3] = [-200.0, 0.0, 200.0];
const CROSS_STREET_HALF_LENGTH_M: f32 = 250.0;
// Covers both frontage rows of a two-lane street without reaching the next street.
const ZONE_BRUSH_RADIUS_M: f64 = 40.0;
const ZONE_BRUSH: CellSelectionShape = CellSelectionShape::Brush {
    radius_m: ZONE_BRUSH_RADIUS_M,
};
// Keeps paint clear of junction corners, where frontage cells do not form.
const JUNCTION_CLEARANCE_M: f64 = 30.0;

/// City state at one daily settlement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrowthDayRecord {
    /// Operational day that the settlement opened.
    pub day: u32,
    /// Households with at least one member.
    pub households: u32,
    /// Resident agents, children included.
    pub residents: u32,
    /// Residents holding a job.
    pub employed: u32,
    /// Commercial and industrial job capacity of standing buildings.
    pub jobs: u32,
    /// Placed buildings, including those under construction.
    pub buildings: u32,
    /// City treasury balance after settlement.
    pub treasury: f64,
}

/// A small zoned town that grows only through simulated demand.
pub struct GrowthScenario {
    core: SimCore,
}

impl GrowthScenario {
    /// Builds a flat 2 km world with an `OWA` border road, a main street with three cross
    /// streets, residential and industrial paint on the cross streets and commercial paint on
    /// the main street. The clock starts at day 1, 07:30, at maximum speed.
    ///
    /// Panics if the bootstrap pack is missing or a scripted edit is rejected; both mean the
    /// fixture no longer matches the tools it drives.
    pub fn starter_town() -> Self {
        let mut core = SimCore::new(WorldConfig::new(WORLD_SIZE_M, WORLD_SIZE_M, 40.0, 10.0));
        core.precompute_road_mesh_data();
        install_bootstrap_assets(&mut core);

        let border_x = BORDER_INSET_M - core.heightmap.half_world_extents().0;
        commit_road(&mut core, (border_x, 0.0), (MAIN_STREET_END_X_M, 0.0));
        for x in CROSS_STREET_X_M {
            commit_road(
                &mut core,
                (x, -CROSS_STREET_HALF_LENGTH_M),
                (x, CROSS_STREET_HALF_LENGTH_M),
            );
        }
        let border_y = core.heightmap.sample_height_world(border_x, 0.0);
        let border = core.check_border_candidate_internal(Vector3::new(border_x, border_y, 0.0));
        assert!(border >= 0, "the trunk must end at the world edge");
        core.set_border_connection_internal(border as i32);

        let [residential_west, residential_center, industrial_east] =
            CROSS_STREET_X_M.map(f64::from);
        for x in [residential_west, residential_center] {
            paint_cross_street(&mut core, x, ZoneType::Residential);
        }
        paint_cross_street(&mut core, industrial_east, ZoneType::Industrial);
        for (start, end) in [(-170.0, -30.0), (30.0, 170.0)] {
            paint(
                &mut core,
                ZoneType::Commercial,
                ZONE_BRUSH,
                &[DVec2::new(start, 0.0), DVec2::new(end, 0.0)],
            );
        }

        core.time.speed_multiplier = SIMULATION_SPEED_STEPS[SIMULATION_SPEED_STEPS.len() - 1];
        Self { core }
    }

    /// Current operational day.
    pub fn day(&self) -> u32 {
        self.core.time.day_index
    }

    /// Runs `days` daily settlements frame by frame and records the city after each one.
    pub fn run_days(&mut self, days: u32) -> Vec<GrowthDayRecord> {
        let mut records = Vec::with_capacity(days as usize);
        while records.len() < days as usize {
            if self.core.step_frame(FRAME_DT_S).daily_ticks > 0 {
                records.push(self.record());
            }
        }
        records
    }

    fn record(&self) -> GrowthDayRecord {
        let city = self.core.daily_city_flow_diagnostics();
        GrowthDayRecord {
            day: self.core.time.day_index,
            households: city.active_households,
            residents: city.resident_agents,
            employed: city.employed_agents,
            jobs: city
                .commercial_job_capacity
                .saturating_add(city.industrial_job_capacity),
            buildings: self.core.allocator.buildings.len() as u32,
            treasury: self.core.treasury.balance,
        }
    }
}

pub(super) fn install_bootstrap_assets(core: &mut SimCore) {
    let mods = Path::new(env!("CARGO_MANIFEST_DIR")).join("../godot/bootstrap/mods");
    let scan = scan_pack_dir(&mods);
    assert!(
        scan.warnings.is_empty() && !scan.packs.is_empty(),
        "bootstrap packs in {}: {:?}",
        mods.display(),
        scan.warnings
    );
    core.replace_asset_registry(scan.packs.into_iter().flat_map(|pack| {
        let pack_id = pack.pack.pack_id;
        pack.assets
            .into_iter()
            .map(move |(manifest, dir)| (pack_id.clone(), manifest, dir))
    }));
}

pub(super) fn commit_road(core: &mut SimCore, (x0, z0): (f32, f32), (x1, z1): (f32, f32)) {
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(x0, 0.0, z0), Vector3::new(x1, 0.0, z1)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            edit_plan: None,
        },
        Instant::now(),
        Default::default(),
    );
    assert!(
        outcome.committed,
        "road ({x0}, {z0}) -> ({x1}, {z1}) rejected: {}",
        outcome.rejection
    );
}

fn paint_cross_street(core: &mut SimCore, x: f64, zone_type: ZoneType) {
    let reach = f64::from(CROSS_STREET_HALF_LENGTH_M) - ZONE_BRUSH_RADIUS_M;
    for (start, end) in [
        (-reach, -JUNCTION_CLEARANCE_M),
        (JUNCTION_CLEARANCE_M, reach),
    ] {
        paint(
            core,
            zone_type,
            ZONE_BRUSH,
            &[DVec2::new(x, start), DVec2::new(x, end)],
        );
    }
}

pub(super) fn paint(
    core: &mut SimCore,
    zone_type: ZoneType,
    shape: CellSelectionShape,
    path: &[DVec2],
) {
    let profile = core
        .zoning
        .profiles
        .runtime_id_for_zone_density(zone_type, ZoneDensity::Low)
        .expect("baseline zoning profile");
    let preview = core.preview_cell_selection_internal(shape, path);
    assert!(
        core.apply_cell_selection_internal(&preview, profile),
        "{zone_type:?} paint along {path:?} selected no paintable cells"
    );
}
