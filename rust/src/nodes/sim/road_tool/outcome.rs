// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's typed preview outcome (`ROAD-112` round R2-1): what the standing preview of
//! the session's stroke says (valid, pending, the refusal key with its numbers, the cost, the
//! steepest grade, the demolitions, the pose it previews), kept by the bridge beside the
//! dictionary the frame hands `road_tool.gd` to draw. Every decision the bridge takes on its
//! preview (a click sends or refuses, a batch's tier 2 steps, the chips word it, a refusal is
//! logged, a refused commit stays refused) reads this, never a dictionary key.

use std::borrow::Cow;

use godot::builtin::Vector3;

use super::labels::PreviewFacts;
use super::wording::ReasonFacts;
use crate::simulation::network::road_edit::RoadPreviewValidation;

/// The key a stroke the core refused at its commit shows until the pointer or the context
/// moves (the session's rule: the unchanged stroke on the same context stays refused).
pub(crate) const COMMIT_REJECTED_REASON: &str = "road_commit_rejected";

/// Tier 2's state on a resting frame of a batch pose (`preview_road_draft`'s `tier2_state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier2State {
    /// Planning the strokes, or building the accepted batch's overlay.
    Running,
    /// Every stroke planned and the overlay whole.
    Accepted,
    /// A stroke, or the whole batch, refused.
    Refused,
}

impl Tier2State {
    /// The state's key as `preview_road_draft` reports it.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Accepted => "accepted",
            Self::Refused => "refused",
        }
    }
}

/// The verdict and facts of one preview of the session's stroke (a single stroke's, a batch
/// pose's tier 1, the frame its tier 2 finished, or a segmentation's refusal).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PreviewOutcome {
    /// The previewed poses' count when it was made: the hue built with it carries the same
    /// (`ROAD-123` round HF2).
    pub(crate) pose: i64,
    /// The stroke may be built as previewed.
    pub(crate) valid: bool,
    /// A batch whose tier 2 still runs: valid so far, not yet final.
    pub(crate) pending: bool,
    /// The refusal key, empty when valid.
    pub(crate) reason: Cow<'static, str>,
    /// The batch stroke that refused it, if one did.
    pub(crate) draft_stroke: Option<usize>,
    /// The class-priced build cost.
    pub(crate) build_cost: f64,
    /// The built length (a single stroke's prepared length, a batch's strokes' sum).
    pub(crate) length_m: f64,
    /// The road tool's surface generation it was made on.
    pub(crate) surface_generation: i64,
    /// A single stroke's validation: the grade, the warning and a refusal's numbers. `None` for
    /// a batch, whose strokes carry their own.
    pub(crate) validation: Option<RoadPreviewValidation>,
    /// A single stroke's prepared points (the refusal report's location); the buffer is reused
    /// from pose to pose.
    pub(crate) prepared: Vec<Vector3>,
    /// The buildings the stroke demolishes.
    pub(crate) demolish_count: i64,
}

impl PreviewOutcome {
    /// The core refused the sent stroke: this outcome becomes the refusal, so another click on
    /// the unchanged stroke on the same context is refused again instead of sent again.
    pub(crate) fn reject(&mut self) {
        self.valid = false;
        self.pending = false;
        self.reason = Cow::Borrowed(COMMIT_REJECTED_REASON);
    }

    /// A blank outcome keeping this one's prepared-points buffer (the next pose's, so a pose
    /// allocates none once it has grown).
    pub(crate) fn recycled(self) -> Self {
        let mut prepared = self.prepared;
        prepared.clear();
        Self {
            prepared,
            ..Self::default()
        }
    }

    /// What the chips show of it; `pending` while the sent stroke waits.
    pub(crate) fn chip_facts(&self, pending: bool) -> PreviewFacts {
        PreviewFacts {
            refused: !self.valid,
            pending,
            build_cost: Some(self.build_cost),
            max_grade: self.validation.as_ref().map(|v| f64::from(v.max_grade)),
            warning: !self.warning_reason().is_empty() || self.demolish_count > 0,
        }
    }

    /// The refusal's numbers as the notice words them.
    pub(crate) fn reason_facts(&self) -> ReasonFacts {
        self.validation
            .as_ref()
            .map_or_else(ReasonFacts::default, |v| ReasonFacts {
                clearance_m: f64::from(v.clearance_m),
                required_clearance_m: f64::from(v.required_clearance_m),
                geometry_measured: Some(f64::from(v.geometry_measured)),
                geometry_limit: f64::from(v.geometry_limit),
            })
    }

    /// A grade over the design tier the preview warns about, with that tier.
    pub(crate) fn steep(&self) -> Option<(f64, f64)> {
        self.validation
            .as_ref()
            .filter(|v| v.warning_reason == "road_steep")
            .map(|v| (f64::from(v.max_grade), f64::from(v.warning_limit)))
    }

    /// The numbers a refusal report gives: a single stroke's validation, else none (zeros, no
    /// snapped ends).
    pub(crate) fn report_numbers(&self) -> RoadPreviewValidation {
        self.validation.clone().unwrap_or(RoadPreviewValidation {
            allowed_grade: 0.0,
            ..RoadPreviewValidation::valid(0.0)
        })
    }

    fn warning_reason(&self) -> &'static str {
        self.validation.as_ref().map_or("", |v| v.warning_reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejected_outcome_keeps_its_facts_and_refuses() {
        let mut outcome = PreviewOutcome {
            pose: 7,
            valid: true,
            build_cost: 120.0,
            validation: Some(RoadPreviewValidation::valid(0.04)),
            ..PreviewOutcome::default()
        };
        outcome.reject();
        assert!(!outcome.valid && !outcome.pending);
        assert_eq!(outcome.reason, COMMIT_REJECTED_REASON);
        assert_eq!((outcome.pose, outcome.build_cost), (7, 120.0));
        let facts = outcome.chip_facts(false);
        assert!(facts.refused);
        assert_eq!(facts.max_grade, Some(f64::from(0.04_f32)));
    }

    #[test]
    fn a_batch_outcome_reports_no_numbers() {
        let outcome = PreviewOutcome::default();
        let numbers = outcome.report_numbers();
        assert_eq!(numbers.allowed_grade, 0.0);
        assert_eq!(numbers.start_endpoint_snapped_node_id, -1);
        assert_eq!(outcome.reason_facts(), ReasonFacts::default());
        assert_eq!(outcome.chip_facts(true).max_grade, None);
    }
}
