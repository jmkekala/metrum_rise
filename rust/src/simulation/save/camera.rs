// SPDX-License-Identifier: GPL-2.0-only

//! Optional orbit-camera presentation state stored with a city snapshot.

use super::{SaveLoadError, SaveLoadResult};
use rusqlite::{Connection, OptionalExtension, params};

/// Camera controls needed to reconstruct the saved view without losing its orbit pivot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SavedCameraState {
    /// World-space focus point, including its resolved terrain clearance.
    pub pivot: [f32; 3],
    /// Horizontal orbit angle in radians.
    pub yaw: f32,
    /// Vertical orbit angle in radians.
    pub pitch: f32,
    /// Distance from the focus point in metres; also determines orthographic zoom.
    pub distance: f32,
    /// Whether the saved view uses orthographic projection.
    pub orthogonal: bool,
}

impl SavedCameraState {
    /// Rejects non-finite controls and non-positive orbit distances before applying or saving them.
    pub(crate) fn validate(&self) -> SaveLoadResult<()> {
        if self.pivot.iter().all(|value| value.is_finite())
            && self.yaw.is_finite()
            && self.pitch.is_finite()
            && self.distance.is_finite()
            && self.distance > 0.0
        {
            Ok(())
        } else {
            Err(SaveLoadError::custom("invalid saved camera state"))
        }
    }
}

/// Writes the optional presentation row inside the caller's snapshot transaction.
pub(super) fn save_camera_state(conn: &Connection, state: SavedCameraState) -> SaveLoadResult<()> {
    conn.execute(
        "INSERT INTO camera_state(singleton, pivot_x, pivot_y, pivot_z, yaw, pitch, distance, orthogonal) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![state.pivot[0], state.pivot[1], state.pivot[2], state.yaw, state.pitch, state.distance, state.orthogonal],
    )?;
    Ok(())
}

/// Reads and validates the presentation row; camera-less snapshots have no row.
pub(super) fn load_camera_state(conn: &Connection) -> SaveLoadResult<Option<SavedCameraState>> {
    let state = conn
        .query_row(
            "SELECT pivot_x, pivot_y, pivot_z, yaw, pitch, distance, orthogonal FROM camera_state WHERE singleton = 1",
            [],
            |row| {
                Ok(SavedCameraState {
                    pivot: [row.get(0)?, row.get(1)?, row.get(2)?],
                    yaw: row.get(3)?,
                    pitch: row.get(4)?,
                    distance: row.get(5)?,
                    orthogonal: row.get(6)?,
                })
            },
        )
        .optional()?;
    if let Some(state) = state {
        state.validate()?;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_state_is_optional_and_rejects_invalid_controls() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::simulation::save::schema::SCHEMA)
            .unwrap();
        assert_eq!(load_camera_state(&conn).unwrap(), None);
        let state = SavedCameraState {
            pivot: [2400.25, 98.5, -8440.75],
            yaw: 2.35,
            pitch: -0.4,
            distance: 187.5,
            orthogonal: true,
        };
        save_camera_state(&conn, state).unwrap();
        assert_eq!(load_camera_state(&conn).unwrap(), Some(state));
        conn.execute("UPDATE camera_state SET distance = -1", [])
            .unwrap();
        assert!(load_camera_state(&conn).is_err());
        assert!(
            SavedCameraState {
                yaw: f32::NAN,
                ..state
            }
            .validate()
            .is_err()
        );
        assert!(
            SavedCameraState {
                pivot: [f32::INFINITY, 0.0, 0.0],
                ..state
            }
            .validate()
            .is_err()
        );
    }
}
