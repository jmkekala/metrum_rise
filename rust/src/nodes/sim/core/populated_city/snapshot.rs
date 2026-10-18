// SPDX-License-Identifier: GPL-2.0-only

//! Reusable populated-city SQLite fixtures for short economy performance checks.

use super::PopulatedCity;
use crate::nodes::sim::core::scenario::install_bootstrap_assets;
use crate::nodes::sim::core::state::SimCore;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::SIMULATION_SPEED_STEPS;
use crate::simulation::save::schema::SAVE_VERSION;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// Bump when the revision inputs or their hashing algorithm changes. The source and bootstrap
// manifests are hashed below; simulation changes that alter a fixture must also touch one of the
// included sources or bump this marker explicitly. The save format is checked independently.
const FIXTURE_ALGORITHM_REVISION: &[u8] = b"perf-fixture-revision-v5";
// Both tiers exercise rebuilding in the following day; day 7 had no execution census.
const SNAPSHOT_DAY: u32 = 12;
const SNAPSHOT_MINUTE: u16 = 0;

#[derive(Clone, Copy)]
enum Fixture {
    Baseline,
    FiscalStress,
}

impl Fixture {
    fn day(self) -> u32 {
        match self {
            Self::Baseline => SNAPSHOT_DAY,
            Self::FiscalStress => 16,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Baseline => "populated_city",
            Self::FiscalStress => "fiscal_stress",
        }
    }
}

/// Ensures a valid tier snapshot exists and returns its generation time when it was rebuilt.
pub(super) fn ensure_snapshot(residents: u32, cache_dir: &Path) -> Option<Duration> {
    ensure_fixture(residents, cache_dir, Fixture::Baseline)
}

/// Prepares a separately validated hourly-churn fixture without replacing the baseline workload.
pub(super) fn ensure_fiscal_snapshot(residents: u32, cache_dir: &Path) -> Option<Duration> {
    ensure_fixture(residents, cache_dir, Fixture::FiscalStress)
}

fn ensure_fixture(residents: u32, cache_dir: &Path, fixture: Fixture) -> Option<Duration> {
    fs::create_dir_all(cache_dir)
        .unwrap_or_else(|error| panic!("could not create performance snapshot directory: {error}"));
    let path = snapshot_path(residents, cache_dir, fixture);
    let fixture_revision = fixture_revision();
    if snapshot_metadata_matches(&path, residents, &fixture_revision, fixture) {
        return None;
    }

    let started = Instant::now();
    let mut city = match fixture {
        Fixture::Baseline => PopulatedCity::build_timed(residents).0,
        Fixture::FiscalStress => fiscal_stress_city(residents, cache_dir),
    };
    warm_to_snapshot_boundary(&mut city, fixture);
    let temporary_path = temporary_snapshot_path(residents, cache_dir, fixture);
    city.core
        .save_game_internal(
            temporary_path
                .to_str()
                .expect("performance snapshot path must be UTF-8"),
            None,
        )
        .unwrap_or_else(|error| panic!("could not write populated-city snapshot: {error}"));
    annotate_snapshot(
        &temporary_path,
        residents,
        city.departures,
        &fixture_revision,
        fixture,
    );

    // A cache is accepted only after a fresh load produces the same next daily record as the
    // straight-through city. This catches omitted continuation state before the fixture is used.
    let mut loaded = load_snapshot_from_path(residents, &temporary_path, fixture);
    let building_revision = city.core.allocator.building_ref_revision();
    assert_exact_continuation(&mut city, &mut loaded, fixture);
    // At 10k the healthy city has almost no failed businesses to rebuild since settlement moved to
    // each building's own phase (`SIM-02-G`), so the 30k tier carries this check.
    if residents == 30_000 {
        assert_ne!(
            city.core.allocator.building_ref_revision(),
            building_revision,
            "performance snapshot must exercise building mutations during its next day",
        );
    }
    fs::rename(&temporary_path, &path).unwrap_or_else(|error| {
        panic!(
            "could not publish populated-city snapshot {}: {error}",
            path.display()
        )
    });
    Some(started.elapsed())
}

/// Loads a previously validated populated-city snapshot.
pub(super) fn load_snapshot(residents: u32, cache_dir: &Path) -> PopulatedCity {
    load_fixture(residents, cache_dir, Fixture::Baseline)
}

/// Loads the separately revision-checked bankruptcy workload for hourly mutation coverage.
pub(super) fn load_fiscal_snapshot(residents: u32, cache_dir: &Path) -> PopulatedCity {
    load_fixture(residents, cache_dir, Fixture::FiscalStress)
}

/// Applies the ordinary policy change used to prepare the bankruptcy workload.
pub(super) fn fiscal_stress_city(residents: u32, cache_dir: &Path) -> PopulatedCity {
    let mut city = load_snapshot(residents, cache_dir);
    for policy in [
        crate::simulation::economy::fiscal::POLICY_COMMERCIAL_PROPERTY_TAX,
        crate::simulation::economy::fiscal::POLICY_INDUSTRIAL_PROPERTY_TAX,
    ] {
        assert!(city.core.set_fiscal_policy_value(policy, 500.0));
    }
    city
}

fn load_fixture(residents: u32, cache_dir: &Path, fixture: Fixture) -> PopulatedCity {
    let path = snapshot_path(residents, cache_dir, fixture);
    let fixture_revision = fixture_revision();
    assert!(
        snapshot_metadata_matches(&path, residents, &fixture_revision, fixture),
        "populated-city snapshot is missing or stale: {}",
        path.display()
    );
    load_snapshot_from_path(residents, &path, fixture)
}

fn snapshot_path(residents: u32, cache_dir: &Path, fixture: Fixture) -> PathBuf {
    cache_dir.join(format!("{}_{residents}.sqlite", fixture.name()))
}

fn temporary_snapshot_path(residents: u32, cache_dir: &Path, fixture: Fixture) -> PathBuf {
    cache_dir.join(format!(
        ".{}_{residents}.{}.tmp.sqlite",
        fixture.name(),
        std::process::id()
    ))
}

fn annotate_snapshot(
    path: &Path,
    residents: u32,
    departures: u32,
    fixture_revision: &str,
    fixture: Fixture,
) {
    let mut connection = Connection::open(path)
        .unwrap_or_else(|error| panic!("could not reopen populated-city snapshot: {error}"));
    let transaction = connection
        .transaction()
        .unwrap_or_else(|error| panic!("could not annotate populated-city snapshot: {error}"));
    transaction
        .execute_batch(
            "CREATE TABLE perf_fixture_meta(
                fixture_revision TEXT NOT NULL,
                residents INTEGER NOT NULL,
                day_index INTEGER NOT NULL,
                minute_of_day INTEGER NOT NULL,
                departures INTEGER NOT NULL
            );",
        )
        .unwrap_or_else(|error| panic!("could not create populated-city metadata: {error}"));
    transaction
        .execute(
            "INSERT INTO perf_fixture_meta(fixture_revision, residents, day_index, minute_of_day, departures) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                fixture_revision,
                i64::from(residents),
                i64::from(fixture.day()),
                i64::from(SNAPSHOT_MINUTE),
                i64::from(departures),
            ],
        )
        .unwrap_or_else(|error| panic!("could not write populated-city metadata: {error}"));
    transaction
        .commit()
        .unwrap_or_else(|error| panic!("could not commit populated-city metadata: {error}"));
}

fn snapshot_metadata_matches(
    path: &Path,
    residents: u32,
    fixture_revision: &str,
    fixture: Fixture,
) -> bool {
    let Ok(connection) =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return false;
    };
    let save_version = connection
        .query_row("SELECT version FROM save_meta LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .ok();
    let metadata = connection
        .query_row(
            "SELECT fixture_revision, residents, day_index, minute_of_day FROM perf_fixture_meta LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .ok();
    save_version == Some(SAVE_VERSION)
        && metadata
            == Some((
                fixture_revision.to_owned(),
                i64::from(residents),
                i64::from(fixture.day()),
                i64::from(SNAPSHOT_MINUTE),
            ))
}

fn warm_to_snapshot_boundary(city: &mut PopulatedCity, fixture: Fixture) {
    city.core.time.speed_multiplier = SIMULATION_SPEED_STEPS[SIMULATION_SPEED_STEPS.len() - 1];
    while city.core.time.day_index < fixture.day() {
        city.advance_hour();
    }
    assert_eq!(
        (city.core.time.day_index, city.core.time.minute_of_day),
        (fixture.day(), SNAPSHOT_MINUTE)
    );
}

fn load_snapshot_from_path(residents: u32, path: &Path, fixture: Fixture) -> PopulatedCity {
    let metadata = read_departures(path, residents, fixture);
    let mut core = SimCore {
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::new(2048.0, 2048.0, 40.0, 10.0))
    };
    install_bootstrap_assets(&mut core);
    core.load_game_internal(
        path.to_str()
            .expect("performance snapshot path must be UTF-8"),
    )
    .unwrap_or_else(|error| panic!("could not load populated-city snapshot: {error}"));
    assert_eq!(
        (core.time.day_index, core.time.minute_of_day),
        (fixture.day(), SNAPSHOT_MINUTE),
        "saved clock does not match the fixture boundary",
    );
    core.time.speed_multiplier = SIMULATION_SPEED_STEPS[SIMULATION_SPEED_STEPS.len() - 1];
    PopulatedCity {
        core,
        departures: metadata,
    }
}

fn read_departures(path: &Path, residents: u32, fixture: Fixture) -> u32 {
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|error| panic!("could not open populated-city snapshot: {error}"));
    let metadata: (String, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT fixture_revision, residents, day_index, minute_of_day, departures FROM perf_fixture_meta LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap_or_else(|error| panic!("invalid populated-city metadata: {error}"));
    assert_eq!(metadata.0, fixture_revision());
    assert_eq!(metadata.1, i64::from(residents));
    assert_eq!(metadata.2, i64::from(fixture.day()));
    assert_eq!(metadata.3, i64::from(SNAPSHOT_MINUTE));
    u32::try_from(metadata.4).expect("snapshot departures must fit u32")
}

fn fixture_revision() -> String {
    fixture_revision_at(&Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
}

fn fixture_revision_at(repository: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FIXTURE_ALGORITHM_REVISION);
    for source in [
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/nodes/sim/core/populated_city.rs"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/nodes/sim/core/populated_city/snapshot.rs"
        ))
        .as_slice(),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/nodes/sim/core/scenario.rs"
        ))
        .as_slice(),
    ] {
        hasher.update(source.len().to_le_bytes());
        hasher.update(source);
    }

    let root = repository.join("godot/bootstrap/mods");
    // These definitions are loaded from disk independently of the bootstrap asset registry.
    // Their contents affect warm-up, so an asset-only revision can accept a stale city.
    let mut manifests = vec![
        repository.join("economy/profiles.toml"),
        repository.join("demand/growth_profiles.toml"),
    ];
    collect_bootstrap_manifests(&root, &mut manifests);
    manifests.sort();
    for path in manifests {
        let relative = path
            .strip_prefix(repository)
            .expect("fixture input must be below repository root");
        let name = relative.to_string_lossy();
        hasher.update(name.len().to_le_bytes());
        hasher.update(name.as_bytes());
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!("could not read fixture input {}: {error}", path.display())
        });
        hasher.update(bytes.len().to_le_bytes());
        hasher.update(bytes);
    }
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn collect_bootstrap_manifests(root: &Path, manifests: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root).unwrap_or_else(|error| {
        panic!(
            "could not read bootstrap directory {}: {error}",
            root.display()
        )
    });
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("could not inspect bootstrap directory: {error}"))
            .path();
        if path.is_dir() {
            collect_bootstrap_manifests(&path, manifests);
        } else if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("json" | "toml")
        ) {
            manifests.push(path);
        }
    }
}

#[test]
fn snapshot_revision_rejects_changed_economy_demand_and_assets() {
    let root = std::env::temp_dir().join(format!(
        "metrum_fixture_revision_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    let inputs = [
        "economy/profiles.toml",
        "demand/growth_profiles.toml",
        "godot/bootstrap/mods/test/manifest.json",
    ];
    for input in inputs {
        let path = root.join(input);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "original").unwrap();
    }
    let revision = fixture_revision_at(&root);
    let saved = root.join("snapshot.sqlite");
    {
        let connection = Connection::open(&saved).unwrap();
        connection
            .execute("CREATE TABLE save_meta(version INTEGER)", [])
            .unwrap();
        connection
            .execute("INSERT INTO save_meta VALUES (?1)", [SAVE_VERSION])
            .unwrap();
    }
    annotate_snapshot(&saved, 10_000, 0, &revision, Fixture::Baseline);
    assert!(snapshot_metadata_matches(
        &saved,
        10_000,
        &revision,
        Fixture::Baseline
    ));
    assert!(!snapshot_metadata_matches(
        &saved,
        10_000,
        &revision,
        Fixture::FiscalStress
    ));
    assert!(!snapshot_metadata_matches(
        &saved,
        30_000,
        &revision,
        Fixture::Baseline
    ));
    for input in inputs {
        fs::write(root.join(input), "changed").unwrap();
        let changed = fixture_revision_at(&root);
        assert_ne!(revision, changed, "untracked fixture input: {input}");
        assert!(!snapshot_metadata_matches(
            &saved,
            10_000,
            &changed,
            Fixture::Baseline
        ));
        fs::write(root.join(input), "original").unwrap();
        assert_eq!(revision, fixture_revision_at(&root));
    }
    // Identical bytes under another asset path still describe a different registry input.
    fs::rename(
        root.join(inputs[2]),
        root.join("godot/bootstrap/mods/test/other.json"),
    )
    .unwrap();
    assert_ne!(revision, fixture_revision_at(&root));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bulk_fixture_lane_geometry_matches_loaded_rebuild() {
    let (mut core, _, _) = super::street_grid(400.0);
    let geometry = |core: &SimCore| {
        core.transit_network
            .lane_system
            .lanes
            .iter()
            .map(|lane| {
                (
                    lane.length.to_bits(),
                    lane.geometry
                        .iter()
                        .map(|point| (point.x.to_bits(), point.y.to_bits(), point.z.to_bits()))
                        .collect::<Vec<_>>(),
                    lane.cum_dist
                        .iter()
                        .map(|value| value.to_bits())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    let before = geometry(&core);
    // Match ordinary loading's lane rebuild and height fit without populating a city.
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.transit_network
        .lane_system
        .sync_heights_to_visible_surface(
            &core.region_graph,
            &core.heightmap,
            &core.transit_network.road_surface,
        );
    assert_eq!(before, geometry(&core));
}

#[test]
#[ignore = "small release-only snapshot continuation regression"]
// Runtime guard: the ignored check must still compile in a debug test build.
#[allow(clippy::assertions_on_constants)]
fn small_snapshot_continuation_round_trip() {
    assert!(
        !cfg!(debug_assertions),
        "run the representative snapshot check in a release test build"
    );
    let cache_dir = std::env::temp_dir().join(format!(
        "metrum_rise_perf_snapshot_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must be after Unix epoch")
            .as_nanos()
    ));
    // Two cross-street columns are the smallest non-degenerate populated-city layout.
    let generation = ensure_snapshot(500, &cache_dir);
    assert!(generation.is_some());
    let city = load_snapshot(500, &cache_dir);
    assert_eq!(
        (city.core.time.day_index, city.core.time.minute_of_day),
        (SNAPSHOT_DAY, SNAPSHOT_MINUTE)
    );
    fs::remove_dir_all(&cache_dir).expect("remove temporary performance snapshot directory");
}

fn assert_exact_continuation(
    straight: &mut PopulatedCity,
    loaded: &mut PopulatedCity,
    fixture: Fixture,
) {
    use crate::debug::work::{Work, snapshot as work_snapshot};
    assert_eq!(
        straight.record(),
        loaded.record(),
        "snapshot record changed during loading"
    );
    let target_day = straight.core.time.day_index + 1;
    for hour in 1..=24 {
        let mut churn = [[0; 2]; 2];
        for (city, row) in [&mut *straight, &mut *loaded].into_iter().zip(&mut churn) {
            let before = work_snapshot();
            // Each window holds one hour's building actions, at its actions slot (`SIM-02-I`).
            city.advance_hour_through_actions();
            let after = work_snapshot();
            for (slot, counter) in row
                .iter_mut()
                .zip([Work::BuildingsPlaced, Work::BuildingsRemoved])
            {
                *slot = after[counter as usize] - before[counter as usize];
            }
        }
        assert_eq!(
            churn[0], churn[1],
            "snapshot churn differs at continuation hour {hour}"
        );
        if matches!(fixture, Fixture::FiscalStress) {
            assert!(
                churn[0].iter().sum::<u64>() > 0,
                "no building churn at continuation hour {hour}"
            );
        }
        assert_eq!(
            straight.record(),
            loaded.record(),
            "snapshot record differs at continuation hour {hour}"
        );
        assert_eq!(
            (
                straight.core.time.day_index,
                straight.core.time.minute_of_day
            ),
            (loaded.core.time.day_index, loaded.core.time.minute_of_day),
            "snapshot clocks differ at continuation hour {hour}",
        );
        assert_eq!(
            first_movement_difference(straight, loaded),
            None,
            "snapshot agent state differs at continuation hour {hour}",
        );
    }
    assert_eq!(straight.core.time.day_index, target_day);
    assert_eq!(
        straight.record(),
        loaded.record(),
        "populated-city snapshot did not produce the same next-day record",
    );
}

fn first_movement_difference(
    straight: &PopulatedCity,
    loaded: &PopulatedCity,
) -> Option<(usize, String, String)> {
    let count = straight.core.agents.len().min(loaded.core.agents.len());
    for index in 0..count {
        let a = &straight.core.agents;
        let b = &loaded.core.agents;
        if a.home_building[index] != b.home_building[index]
            || a.transit[index] != b.transit[index]
            || a.transit_mode[index] != b.transit_mode[index]
            || a.current_node[index] != b.current_node[index]
            || a.current_edge[index] != b.current_edge[index]
            || a.current_lane_id[index] != b.current_lane_id[index]
            || a.lane_distance[index].to_bits() != b.lane_distance[index].to_bits()
            || a.pos_x[index].to_bits() != b.pos_x[index].to_bits()
            || a.pos_y[index].to_bits() != b.pos_y[index].to_bits()
            || a.speed[index].to_bits() != b.speed[index].to_bits()
            || a.current_path_index[index] != b.current_path_index[index]
            || a.current_path[index] != b.current_path[index]
            || a.activity[index] != b.activity[index]
            || a.work_building[index] != b.work_building[index]
            || a.current_building[index] != b.current_building[index]
            || a.target_building[index] != b.target_building[index]
            || a.planned_target_building[index] != b.planned_target_building[index]
            || a.planned_activity[index] != b.planned_activity[index]
            || a.cached_commute_minutes[index] != b.cached_commute_minutes[index]
            || a.next_commute_refresh_time[index].to_bits()
                != b.next_commute_refresh_time[index].to_bits()
            || a.next_departure_day[index] != b.next_departure_day[index]
            || a.next_departure_minute[index] != b.next_departure_minute[index]
            || a.next_departure_origin_building[index] != b.next_departure_origin_building[index]
            || a.next_departure_target_building[index] != b.next_departure_target_building[index]
            || a.next_departure_activity[index] != b.next_departure_activity[index]
        {
            return Some((
                index,
                agent_debug(straight, index),
                agent_debug(loaded, index),
            ));
        }
    }
    (straight.core.agents.len() != loaded.core.agents.len()).then_some((
        count,
        format!("agent_count={}", straight.core.agents.len()),
        format!("agent_count={}", loaded.core.agents.len()),
    ))
}

fn agent_debug(city: &PopulatedCity, index: usize) -> String {
    let agents = &city.core.agents;
    format!(
        "home={} work={} household={} age={} activity={} transit={} mode={} current_building={} target_building={} planned_target={} planned_activity={} current_node={} attach=({}, {}, {:?}) detach=({}, {}, {:?}) access={} next_replan={:?} edge={} lane={} lane_distance={:?} pos=({:?},{:?}) speed={:?} journey={:?} schedule={} commute={} next_commute={:?} departure=({}, {}, {}, {}, {}) work_cache=({}, {}) car={} vehicle={} path_index={} path={:?}",
        agents.home_building[index],
        agents.work_building[index],
        agents.household_id[index],
        agents.age_group[index],
        agents.activity[index],
        agents.transit[index],
        agents.transit_mode[index],
        agents.current_building[index],
        agents.target_building[index],
        agents.planned_target_building[index],
        agents.planned_activity[index],
        agents.current_node[index],
        agents.planned_attach_node[index],
        agents.planned_attach_lane_id[index],
        agents.planned_attach_lane_d[index].to_bits(),
        agents.planned_detach_node[index],
        agents.planned_detach_lane_id[index],
        agents.planned_detach_lane_d[index].to_bits(),
        agents.access_flags[index],
        agents.next_replan_time[index].to_bits(),
        agents.current_edge[index],
        agents.current_lane_id[index],
        agents.lane_distance[index].to_bits(),
        agents.pos_x[index].to_bits(),
        agents.pos_y[index].to_bits(),
        agents.speed[index].to_bits(),
        agents.journey_start_time[index].to_bits(),
        agents.schedule_seed[index],
        agents.cached_commute_minutes[index],
        agents.next_commute_refresh_time[index].to_bits(),
        agents.next_departure_day[index],
        agents.next_departure_minute[index],
        agents.next_departure_origin_building[index],
        agents.next_departure_target_building[index],
        agents.next_departure_activity[index],
        agents.cached_schedule_work_building[index],
        agents.cached_work_profile_index[index],
        agents.has_car[index],
        agents.vehicle_type[index],
        agents.current_path_index[index],
        agents.current_path[index],
    )
}
