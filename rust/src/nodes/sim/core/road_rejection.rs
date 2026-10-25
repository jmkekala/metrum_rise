// SPDX-License-Identifier: GPL-2.0-only

//! Road rejection reports on `events`: why a stroke was refused, where, and its exact inputs.
//!
//! A refused stroke logs two lines. The first explains the refusal for a person; the second,
//! `road_attempt {json}`, carries the submitted points at full precision with lanes, snapping and
//! the road-tool generation, so `run.sh --replay-road-attempts` can replay it against the save the
//! console's `dump` writes. Reports are built only when a stroke is refused.

use crate::debug_log;
use crate::simulation::network::road_edit::RoadPreviewValidation;
use godot::prelude::Vector3;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(1);

/// Which check refused a stroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RejectedBy {
    /// The road tool's click check, before anything reached the simulation.
    Tool,
    /// The simulation's authoritative commit.
    Commit,
}

impl RejectedBy {
    fn label(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Commit => "commit",
        }
    }
}

/// The submitted stroke, exactly as the road tool sent it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RoadAttempt<'a> {
    pub(crate) points: &'a [Vector3],
    pub(crate) fwd_lanes: i32,
    pub(crate) bkw_lanes: i32,
    pub(crate) snap: bool,
}

/// Why one stroke was refused.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RoadRejection {
    /// Stable reason key, such as `bridge_clearance` or `parcel_overlap`.
    pub(crate) reason: String,
    /// The reason in words, with the numbers the check used.
    pub(crate) detail: String,
    /// World position the failed check measured, when it measured one.
    pub(crate) at: Option<Vector3>,
    /// Prepared stroke ends: snapped node or open terrain, road and terrain heights.
    pub(crate) ends: String,
}

impl RoadRejection {
    /// A rejection without validation numbers.
    pub(crate) fn new(reason: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            detail: detail.into(),
            ..Self::default()
        }
    }

    /// A rejection explained from a road validation. `reason` may be a check the validation does
    /// not carry, such as the road tool's parcel and field overlaps.
    pub(crate) fn from_validation(
        reason: &str,
        validation: &RoadPreviewValidation,
        prepared: &[Vector3],
    ) -> Self {
        let (detail, at) = validation_detail(reason, validation, prepared);
        Self {
            reason: reason.to_owned(),
            detail,
            at,
            ends: ends_text(validation),
        }
    }

    /// Adds a clause to the detail, such as a parcel count.
    pub(crate) fn with_note(mut self, note: impl AsRef<str>) -> Self {
        self.detail = format!("{}; {}", self.detail, note.as_ref());
        self
    }

    /// Logs both lines on `events` and returns the attempt number that ties them together.
    pub(crate) fn report(&self, by: RejectedBy, attempt: RoadAttempt<'_>, generation: u64) -> u64 {
        let number = NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed);
        let (why, replay) = self.lines(number, by, attempt, generation);
        debug_log!("events", "{why}");
        debug_log!("events", "{replay}");
        number
    }

    fn lines(
        &self,
        number: u64,
        by: RejectedBy,
        attempt: RoadAttempt<'_>,
        generation: u64,
    ) -> (String, String) {
        let mut why = format!(
            "road rejected by {}: {}: {}",
            by.label(),
            self.reason,
            self.detail
        );
        if let Some(at) = self.at {
            why.push_str(&format!("; at ({:.1}, {:.1})", at.x, at.z));
        }
        why.push_str(&format!(". {}", stroke_text(attempt)));
        if !self.ends.is_empty() {
            why.push_str(&format!("; {}", self.ends));
        }
        why.push_str(&format!(". Attempt {number}."));
        let replay = AttemptRecord {
            attempt: number,
            by: by.label(),
            reason: &self.reason,
            generation,
            fwd_lanes: attempt.fwd_lanes,
            bkw_lanes: attempt.bkw_lanes,
            snap: attempt.snap,
            points: attempt.points.iter().map(|p| [p.x, p.y, p.z]).collect(),
        };
        let json = serde_json::to_string(&replay).expect("attempt records serialize");
        (why, format!("road_attempt {json}"))
    }
}

// `f32` fields serialize in their shortest round-trip form, so a replay rebuilds the same points.
#[derive(Serialize)]
struct AttemptRecord<'a> {
    attempt: u64,
    by: &'static str,
    reason: &'a str,
    generation: u64,
    fwd_lanes: i32,
    bkw_lanes: i32,
    snap: bool,
    points: Vec<[f32; 3]>,
}

fn validation_detail(
    reason: &str,
    validation: &RoadPreviewValidation,
    prepared: &[Vector3],
) -> (String, Option<Vector3>) {
    match reason {
        "bridge_clearance" | "tunnel_clearance" if validation.required_clearance_m <= 0.0 => (
            format!(
                "the bridge profile dips {:.2} m below the terrain",
                -validation.clearance_m
            ),
            None,
        ),
        // Both validators measure clearance at the middle of the prepared profile.
        "bridge_clearance" | "tunnel_clearance" => (
            format!(
                "clearance {:.2} m at the middle of the road, needs {:.2} m",
                validation.clearance_m, validation.required_clearance_m
            ),
            prepared.get(prepared.len() / 2).copied(),
        ),
        "same_node_connection" => (
            format!(
                "both ends snap to node {}",
                validation.start_endpoint_snapped_node_id
            ),
            None,
        ),
        "too_short" if prepared.len() < 2 => (
            "the stroke collapses to a single point once prepared".to_owned(),
            None,
        ),
        "too_short" => (
            format!(
                "the prepared road is {:.2} m long, too short to build",
                polyline_length_m(prepared)
            ),
            None,
        ),
        "water_requires_bridge" => (
            "the grounded road corridor crosses visible water; build it as a bridge".to_owned(),
            None,
        ),
        "junction_corner_does_not_fit" => (
            "a junction corner does not fit within half of an arm, even at its minimum radius"
                .to_owned(),
            None,
        ),
        "junction_core_not_simple" => (
            "a junction's carriageway outline crosses itself".to_owned(),
            None,
        ),
        "junction_band_mismatch" => (
            "the cross-sections meeting at a junction do not match (footpaths join roads in \
             ROAD-68-F)"
                .to_owned(),
            None,
        ),
        "junction_arm_too_short" => (
            "an arm is too short for its junction's minimum extent and crosswalk".to_owned(),
            None,
        ),
        "junction_profile_mismatch" => (
            "a solved road centreline misses its junction plane by more than 1 mm".to_owned(),
            None,
        ),
        "field_overlap" => ("the road corridor overlaps a farm field".to_owned(), None),
        "parcel_overlap" => ("the road corridor overlaps zoned parcels".to_owned(), None),
        "validation_unavailable" => (
            "the road check had no result for this stroke yet".to_owned(),
            None,
        ),
        "road_too_steep" => (
            format!(
                "the road would climb at {:.0}%, steeper than the {:.0}% (1:3) maximum grade",
                validation.geometry_measured * 100.0,
                validation.geometry_limit * 100.0
            ),
            None,
        ),
        "road_commit_rejected" => (
            "the simulation refused this stroke earlier; see that commit report".to_owned(),
            None,
        ),
        _ if reason == "surface_geometry_invalid" || reason.starts_with("terrain_clip") => (
            format!("the road surface could not be compiled ({reason})"),
            None,
        ),
        _ => (format!("the {reason} check failed"), None),
    }
}

fn ends_text(validation: &RoadPreviewValidation) -> String {
    let end = |node: i32, height: f32, terrain: f32| {
        let at = if node >= 0 {
            format!("node {node}")
        } else {
            "open terrain".to_owned()
        };
        format!("{at}, road y {height:.2}, terrain y {terrain:.2}")
    };
    format!(
        "start {}; end {}",
        end(
            validation.start_endpoint_snapped_node_id,
            validation.start_endpoint_height_m,
            validation.start_endpoint_terrain_height_m
        ),
        end(
            validation.end_endpoint_snapped_node_id,
            validation.end_endpoint_height_m,
            validation.end_endpoint_terrain_height_m
        )
    )
}

fn stroke_text(attempt: RoadAttempt<'_>) -> String {
    let (Some(first), Some(last)) = (attempt.points.first(), attempt.points.last()) else {
        return "Stroke has no points".to_owned();
    };
    format!(
        "Stroke ({:.1}, {:.1}) -> ({:.1}, {:.1}), {} points, {:.1} m, lanes {}/{}, snap {}",
        first.x,
        first.z,
        last.x,
        last.z,
        attempt.points.len(),
        polyline_length_m(attempt.points),
        attempt.fwd_lanes,
        attempt.bkw_lanes,
        if attempt.snap { "on" } else { "off" }
    )
}

// A fold from +0.0: an empty float `sum()` is -0.0 and prints as "-0.00".
fn polyline_length_m(points: &[Vector3]) -> f32 {
    points
        .windows(2)
        .fold(0.0, |length, pair| length + pair[0].distance_to(pair[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validation() -> RoadPreviewValidation {
        RoadPreviewValidation {
            is_valid: false,
            invalid_reason: "bridge_clearance",
            max_grade: 0.0,
            allowed_grade: 0.0,
            warning_reason: "",
            warning_limit: 0.0,
            offending_span_start_m: 0.0,
            offending_span_end_m: 0.0,
            offending_span_run_m: 0.0,
            offending_span_height_delta_m: 0.0,
            offending_span_start_height_m: 0.0,
            offending_span_end_height_m: 0.0,
            offending_span_start_terrain_height_m: 0.0,
            offending_span_end_terrain_height_m: 0.0,
            offending_span_start_support_delta_m: 0.0,
            offending_span_end_support_delta_m: 0.0,
            start_endpoint_snapped_node_id: 4,
            end_endpoint_snapped_node_id: -1,
            start_endpoint_height_m: 3.0,
            end_endpoint_height_m: 9.5,
            start_endpoint_terrain_height_m: 3.0,
            end_endpoint_terrain_height_m: 2.25,
            start_endpoint_support_delta_m: 0.0,
            end_endpoint_support_delta_m: 7.25,
            clearance_m: 1.5,
            required_clearance_m: 4.5,
            geometry_measured: 0.0,
            geometry_limit: 0.0,
        }
    }

    #[test]
    fn report_explains_the_check_and_records_exact_inputs() {
        let points = [
            Vector3::new(0.1, 3.0, -2.0),
            Vector3::new(50.0, 6.0, -2.0),
            Vector3::new(100.0, 9.5, 1.0e-3),
        ];
        let rejection = RoadRejection::from_validation("bridge_clearance", &validation(), &points)
            .with_note("checked twice");
        let attempt = RoadAttempt {
            points: &points,
            fwd_lanes: 2,
            bkw_lanes: 1,
            snap: true,
        };
        let (why, replay) = rejection.lines(7, RejectedBy::Commit, attempt, 42);
        assert_eq!(
            why,
            "road rejected by commit: bridge_clearance: clearance 1.50 m at the middle of the \
             road, needs 4.50 m; checked twice; at (50.0, -2.0). Stroke (0.1, -2.0) -> \
             (100.0, 0.0), 3 points, 100.2 m, lanes 2/1, snap on; start node 4, road y 3.00, \
             terrain y 3.00; end open terrain, road y 9.50, terrain y 2.25. Attempt 7."
        );
        let json = replay.strip_prefix("road_attempt ").expect("replay prefix");
        let record: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(record["attempt"], 7);
        assert_eq!(record["by"], "commit");
        assert_eq!(record["reason"], "bridge_clearance");
        assert_eq!(record["generation"], 42);
        assert_eq!(
            (record["fwd_lanes"].clone(), record["bkw_lanes"].clone()),
            (2.into(), 1.into())
        );
        assert_eq!(record["snap"], true);
        // Every coordinate parses back to the submitted f32.
        let parsed: Vec<Vec<f64>> = serde_json::from_value(record["points"].clone()).unwrap();
        for (point, row) in points.iter().zip(&parsed) {
            assert_eq!(
                [row[0] as f32, row[1] as f32, row[2] as f32],
                [point.x, point.y, point.z]
            );
        }
    }

    #[test]
    fn reasons_without_numbers_still_explain_themselves() {
        let ground = RoadPreviewValidation {
            required_clearance_m: 0.0,
            clearance_m: -0.4,
            ..validation()
        };
        let (detail, at) = validation_detail("bridge_clearance", &ground, &[]);
        assert_eq!(
            (detail.as_str(), at),
            ("the bridge profile dips 0.40 m below the terrain", None)
        );
        let (detail, _) = validation_detail("same_node_connection", &validation(), &[]);
        assert_eq!(detail, "both ends snap to node 4");
        let (detail, _) = validation_detail("too_short", &validation(), &[Vector3::ZERO]);
        assert_eq!(
            detail,
            "the stroke collapses to a single point once prepared"
        );
        let short = [Vector3::ZERO, Vector3::new(1.5, 0.0, 0.0)];
        let (detail, _) = validation_detail("too_short", &validation(), &short);
        assert_eq!(
            detail,
            "the prepared road is 1.50 m long, too short to build"
        );
        let (detail, _) = validation_detail(
            "terrain_clip_missing_outer_boundary_owner",
            &validation(),
            &[],
        );
        assert!(
            detail.contains("could not be compiled (terrain_clip_missing_outer_boundary_owner)")
        );
        let (detail, _) = validation_detail("stale_surface_generation", &validation(), &[]);
        assert_eq!(detail, "the stale_surface_generation check failed");
        let plain = RoadRejection::new(
            "zoning_cell_overlap",
            "the corridor overlaps painted zoning cells",
        );
        let attempt = RoadAttempt {
            points: &[],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap: false,
        };
        let (why, _) = plain.lines(1, RejectedBy::Tool, attempt, 0);
        assert_eq!(
            why,
            "road rejected by tool: zoning_cell_overlap: the corridor overlaps painted zoning \
             cells. Stroke has no points. Attempt 1."
        );
    }
}
