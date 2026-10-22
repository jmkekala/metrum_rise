// SPDX-License-Identifier: GPL-2.0-only

//! Weather state stored with a city snapshot, so a reload resumes the same sky.

use super::{SaveLoadError, SaveLoadResult};
use crate::simulation::core::weather::{WeatherRecord, WeatherState};
use rusqlite::{Connection, params};

/// Writes the weather row inside the caller's snapshot transaction.
pub(super) fn save_weather_state(conn: &Connection, state: &WeatherState) -> SaveLoadResult<()> {
    let r = state.record();
    conn.execute(
        "INSERT INTO weather_state(singleton, seed, target, from_wind_x, from_wind_z, from_coverage, from_optical_depth, transition_elapsed_s, transition_duration_s, cloud_offset_x, cloud_offset_z, wind_clock_s, automatic, event_index, until_event_h) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            i64::from(r.seed),
            i64::from(r.target),
            r.from[0],
            r.from[1],
            r.from[2],
            r.from[3],
            r.transition_elapsed_s,
            r.transition_duration_s,
            r.cloud_offset_m[0],
            r.cloud_offset_m[1],
            r.wind_clock_s,
            i64::from(r.automatic),
            i64::from(r.event_index),
            r.until_event_h
        ],
    )?;
    conn.execute(
        "INSERT INTO weather_sky(singleton, from_high_cover, to_wind_x, to_wind_z, to_high_cover, high_offset_x, high_offset_z) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            r.from[4],
            r.to_wind[0],
            r.to_wind[1],
            r.to_high_cover,
            r.high_offset_m[0],
            r.high_offset_m[1]
        ],
    )?;
    Ok(())
}

/// Reads and validates the weather rows carried by the current save format.
pub(super) fn load_weather_state(conn: &Connection) -> SaveLoadResult<WeatherState> {
    let (seed, target, event_index, record): (i64, i64, i64, WeatherRecord) = conn.query_row(
        "SELECT seed, target, from_wind_x, from_wind_z, from_coverage, from_optical_depth, transition_elapsed_s, transition_duration_s, cloud_offset_x, cloud_offset_z, wind_clock_s, automatic, event_index, until_event_h FROM weather_state WHERE singleton = 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(12)?,
                WeatherRecord {
                    seed: 0,
                    target: 0,
                    from: [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, 0.0],
                    to_wind: [0.0; 2],
                    to_high_cover: 0.0,
                    transition_elapsed_s: row.get(6)?,
                    transition_duration_s: row.get(7)?,
                    cloud_offset_m: [row.get(8)?, row.get(9)?],
                    high_offset_m: [0.0; 2],
                    wind_clock_s: row.get(10)?,
                    automatic: row.get::<_, i64>(11)? != 0,
                    event_index: 0,
                    until_event_h: row.get(13)?,
                },
            ))
        },
    )?;
    let invalid = || SaveLoadError::custom("invalid saved weather state");
    let mut record = WeatherRecord {
        seed: u32::try_from(seed).map_err(|_| invalid())?,
        target: u8::try_from(target).map_err(|_| invalid())?,
        event_index: u32::try_from(event_index).map_err(|_| invalid())?,
        ..record
    };
    (
        record.from[4],
        record.to_wind,
        record.to_high_cover,
        record.high_offset_m,
    ) = conn.query_row(
        "SELECT from_high_cover, to_wind_x, to_wind_z, to_high_cover, high_offset_x, high_offset_z FROM weather_sky WHERE singleton = 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                [row.get(1)?, row.get(2)?],
                row.get(3)?,
                [row.get(4)?, row.get(5)?],
            ))
        },
    )?;
    WeatherState::from_record(record).ok_or_else(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::core::weather::WeatherScenario;

    #[test]
    fn weather_state_round_trips_and_rejects_corruption() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::simulation::save::schema::SCHEMA)
            .unwrap();
        let mut state = WeatherState::new(u32::MAX);
        state.transition_to(WeatherScenario::Overcast, 600.0);
        state.advance(123.25);
        state.set_automatic(true);
        state.advance_scheduled(9000.0, 60.0);
        save_weather_state(&conn, &state).unwrap();
        assert_eq!(load_weather_state(&conn).unwrap(), state);
        for corruption in [
            "UPDATE weather_state SET target = 5",
            "UPDATE weather_state SET seed = -1",
            "UPDATE weather_state SET transition_elapsed_s = 601.0",
            "UPDATE weather_state SET cloud_offset_x = 65536.0",
            "UPDATE weather_state SET wind_clock_s = -0.5",
            "UPDATE weather_state SET until_event_h = -1.0",
            "UPDATE weather_state SET event_index = -1",
            "UPDATE weather_sky SET to_high_cover = 1.5",
            "UPDATE weather_sky SET to_wind_x = 1.0e9",
            "UPDATE weather_sky SET high_offset_z = -1.0",
        ] {
            let tx = conn.unchecked_transaction().unwrap();
            tx.execute(corruption, []).unwrap();
            assert!(load_weather_state(&tx).is_err(), "{corruption}");
            tx.rollback().unwrap();
        }
    }
}
