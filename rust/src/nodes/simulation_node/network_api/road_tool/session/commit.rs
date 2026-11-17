// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's commit sequence on the bridge (`ROAD-112` round R2-1): a confirming click
//! sends the stroke its standing preview accepts (or logs the preview's refusal), the bridge
//! keeps the request (`nodes::sim::road_tool::commit::CommitSequence`) and answers it itself
//! once the core does (`road_tool_commit_poll`): the session hears the answer, an accepted
//! stroke at the map edge ends the chain and names the border node, a refused one turns the
//! standing preview into `road_commit_rejected` and hands `road_tool.gd` its display. The script
//! only draws what the answer says.

use super::super::preview::RejectionFacts;
use super::*;
use crate::nodes::sim::road_tool::commit::CommitStep;
use crate::nodes::sim::road_tool::outcome::COMMIT_REJECTED_REASON;

#[godot_api(secondary)]
impl SimulationNode {
    /// The sent stroke's answer (the tick asks once a frame while one waits and hands it over as
    /// `road_tool_commit_answer`; `road_tool.gd` asks too when chunks install, as the answer may
    /// arrive in their frame): nil until the core answers it, then the answer, once: `committed`, `phase` and `placed` (the session's, after it),
    /// and for an accepted stroke `generation` (the render generation whose road chunks replace
    /// the preview: it holds until they are installed) and `border_node` (the external
    /// connection candidate at an end, -1 for none; the chain ended there), for a refused one
    /// `graded_epoch` (the preview tiles the commit had adopted, for the renderer to drop) and,
    /// when the stroke is editable again, `rejected` (the dictionary the sent stroke's preview
    /// shows the refusal with: `road_commit_rejected`, its graded tiles handed over at the click
    /// left out, numbered as the next preview's `sequence`) with `points` (the stroke it is
    /// for). Nil at once when nothing waits; allocates nothing until the answer.
    #[func]
    pub fn road_tool_commit_poll(&mut self) -> Variant {
        self.road_tool_take_commit_answer()
            .map_or_else(Variant::nil, |answer| answer.to_variant())
    }

    // `road_tool_commit_poll`'s answer, `None` until it arrives.
    pub(super) fn road_tool_take_commit_answer(&mut self) -> Option<VarDictionary> {
        let request_id = self.road_tool_session.commit.request_id();
        if request_id == 0 {
            return None;
        }
        let answer = self.take_road_commit_answer(request_id)?;
        let (committed, generation, detail) = match &answer {
            Ok(ack) => (ack.committed, ack.generation, ack.detail.as_str()),
            Err(reason) => (false, 0, *reason),
        };
        debug_log!(
            "road",
            "road_commit_result request_id={} committed={} detail={}",
            request_id,
            committed,
            detail
        );
        // The border check reads the sent stroke's ends before the answer clears it.
        let border_node = committed
            .then(|| self.road_tool_border_candidate())
            .flatten();
        let points = PackedVector3Array::from(self.road_tool_session.commit.points());
        let step = {
            let view = self.road_tool.view.read().unwrap();
            let bridge = &mut self.road_tool_session;
            let standing = bridge
                .preview
                .as_mut()
                .map(|standing| &mut standing.outcome);
            bridge.commit.answer(
                &mut bridge.session,
                &view,
                committed,
                generation,
                border_node,
                standing,
            )
        };
        let sent = self.road_tool_session.commit_preview.take();
        let (phase, placed) = self.road_tool_phase_placed();
        let mut result = vdict! {
            "committed": committed,
            "phase": phase,
            "placed": placed,
        };
        match step {
            CommitStep::Committed {
                generation,
                border_node,
            } => {
                result.set("generation", i64::try_from(generation).unwrap_or(i64::MAX));
                result.set("border_node", border_node.unwrap_or(-1));
            }
            CommitStep::Refused {
                graded_epoch,
                rejected,
            } => {
                if self.road_tool_session.preview.is_some() {
                    self.road_tool_session
                        .hue
                        .hue()
                        .set_state(HueState::Refused);
                }
                result.set("graded_epoch", graded_epoch);
                if let Some(sent) = sent.filter(|_| rejected) {
                    let mut display = self.road_tool_rejected_preview(&sent);
                    let tick = &mut self.road_tool_session.tick;
                    tick.sequence += 1;
                    display.set("sequence", tick.sequence);
                    result.set("rejected", display);
                    result.set("points", points);
                }
            }
        }
        Some(result)
    }

    /// The sent stroke's commit waiting for its answer (`road_tool_commit_poll`), 0 for none.
    #[func]
    pub fn road_tool_commit_request_id(&self) -> i64 {
        self.road_tool_session.commit.request_id()
    }

    /// The waiting commit's stroke as sent, empty for none (benchmarks and probes).
    #[func]
    pub fn road_tool_commit_points(&self) -> PackedVector3Array {
        PackedVector3Array::from(self.road_tool_session.commit.points())
    }

    /// The graded preview epoch the waiting commit adopted (its tiles show until its edit
    /// installs, or until it is refused), -1 for none.
    #[func]
    pub fn road_tool_commit_graded_epoch(&self) -> i64 {
        self.road_tool_session.commit.graded_epoch()
    }

    // Sends the confirmed stroke: the preview of exactly it (the standing one, else a fresh one),
    // then the commit when the preview accepts it (a graded stroke's strokes as one batch, and
    // the commit adopts this epoch's graded preview tiles), else the refusal's log line and the
    // session's refusal (the stroke stays editable).
    pub(super) fn road_tool_submit(&mut self, stroke: StrokeRequest) -> VarDictionary {
        let fresh = !self.road_tool_preview_is_current();
        if fresh {
            // A batch pose's ghost from here is not drawn: the stroke is sent.
            self.road_tool_run_preview();
            self.road_tool_hand_preview();
        }
        let preview = (self.road_tool_session.preview.as_ref())
            .expect("a standing preview")
            .dictionary
            .clone();
        let valid = self.road_tool_standing_outcome().valid;
        if !valid {
            self.road_tool_log_refusal(&stroke);
        }
        let send = {
            let view = self.road_tool.view.read().unwrap();
            let bridge = &mut self.road_tool_session;
            let outcome = &bridge.preview.as_ref().expect("a standing preview").outcome;
            CommitSequence::click(&mut bridge.session, &view, outcome)
        };
        let points = PackedVector3Array::from(stroke.points.as_slice());
        let request_id = match (send, stroke.grade) {
            (false, _) => 0,
            (true, None) => self.add_road_with_snap(
                points.clone(),
                i32::from(stroke.spec.fwd_lanes),
                i32::from(stroke.spec.bkw_lanes),
                stroke.snap_to_existing_roads,
            ),
            (true, Some(_)) => self.road_tool_send_draft(),
        };
        if send {
            let graded_epoch = self.release_road_preview_terrain();
            let bridge = &mut self.road_tool_session;
            bridge.commit.sent(request_id, &stroke.points, graded_epoch);
            bridge.commit_preview = Some(preview.clone());
        }
        vdict! {
            "points": points,
            "preview": preview,
            "fresh": fresh,
            "request_id": request_id,
        }
    }

    // The standing preview's outcome; a click previews first, so one stands.
    fn road_tool_standing_outcome(&self) -> &PreviewOutcome {
        &(self.road_tool_session.preview.as_ref())
            .expect("a standing preview")
            .outcome
    }

    // Queues the graded draft's strokes as one batch, one undo step (a single stroke as a batch
    // of one, `add_road_with_profile`'s path), with tier 2's plans and pieces when it finished
    // for exactly these strokes (`add_road_draft`'s rule).
    fn road_tool_send_draft(&mut self) -> i64 {
        let frame = self.road_tool_session.session.frame();
        let strokes = frame.draft.strokes.clone();
        let section = self.road_tool_draft_section(frame);
        let products = (strokes.len() > 1)
            .then(|| {
                let requests = self.road_tool_draft_requests();
                self.road_tool
                    .context
                    .lock()
                    .expect("road tool context lock poisoned")
                    .take_batch_products(&requests)
            })
            .flatten();
        let mut batch = draft_batch(strokes, section);
        batch.preview = products;
        self.send_road_batch(batch)
    }

    // The refusal's log line from the standing outcome: a follow-ground stroke as today; a
    // graded draft's refusing stroke with its profile when the preview names one, else the
    // draft as held ground.
    fn road_tool_log_refusal(&self, stroke: &StrokeRequest) {
        let outcome = self.road_tool_standing_outcome();
        let strokes = &self.road_tool_session.session.frame().draft.strokes;
        let (points, profile) = match (stroke.grade, strokes.as_slice(), outcome.draft_stroke) {
            (None, ..) => (&stroke.points, RoadProfile::FollowGround),
            (Some(_), [single], _) => (&single.points, single.profile),
            (Some(_), parts, Some(index)) if index < parts.len() => {
                (&parts[index].points, parts[index].profile)
            }
            (Some(_), ..) => (&stroke.points, RoadProfile::HeldGround),
        };
        let reason = match outcome.reason.as_ref() {
            "" => "unknown",
            reason => reason,
        };
        self.report_road_tool_rejection(
            RejectionFacts {
                reason,
                numbers: outcome.report_numbers(),
                prepared: &outcome.prepared,
                overlapping_parcels: None,
                surface_generation: outcome.surface_generation,
            },
            points,
            [
                i32::from(stroke.spec.fwd_lanes),
                i32::from(stroke.spec.bkw_lanes),
            ],
            stroke.snap_to_existing_roads,
            profile,
        );
    }

    // The border connection candidate at either end of the waiting commit's stroke.
    fn road_tool_border_candidate(&self) -> Option<i64> {
        let points = self.road_tool_session.commit.points();
        let [first, .., last] = points else {
            return None;
        };
        let start = Instant::now();
        let mut candidate = self.check_border_candidate(*first);
        if candidate < 0 {
            candidate = self.check_border_candidate(*last);
        }
        debug_log!(
            "road",
            "border_check_detail candidate={} total_ms={:.3}",
            candidate,
            start.elapsed().as_secs_f64() * 1000.0
        );
        (candidate >= 0).then_some(candidate)
    }

    // A refused commit's display: the preview it was sent with, refused as
    // `road_commit_rejected` on the current context; its graded tiles were handed over at the
    // click. The one copy of the rule (F6).
    fn road_tool_rejected_preview(&self, sent: &VarDictionary) -> VarDictionary {
        let mut rejected = sent.duplicate_shallow();
        rejected.set("is_valid", false);
        rejected.set("is_pending", false);
        rejected.set("invalid_reason", COMMIT_REJECTED_REASON);
        for key in ["graded_tiles", "graded_removed", "graded_pending"] {
            rejected.remove(key);
        }
        rejected.set("surface_generation", self.road_tool_view_generation());
        rejected
    }
}
