// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's commit sequence (`ROAD-112` round R2-1): what a confirming click does with
//! the stroke the session sent, and what the core's answer does to the session and the standing
//! preview. A valid standing preview sends the stroke; the sequence then holds its request id,
//! its points and the graded preview epoch the commit adopted until the answer. An acceptance
//! ends the chain when the stroke reached the map edge (the border dialog asks about an
//! external connection); a refusal turns the standing preview into `road_commit_rejected`, so
//! the unchanged stroke on the same context stays refused, and leaves the stroke editable unless
//! the player cancelled while it waited. Plain Rust: the bridge (`road_tool_commit_poll`) feeds
//! it the core's answer and the border check, `road_tool.gd` only draws what it returns.

use godot::builtin::Vector3;

use super::frame::Phase;
use super::outcome::PreviewOutcome;
use super::session::{RoadToolAction, RoadToolEvent, RoadToolSession};
use crate::nodes::sim::core::RoadToolQuerySnapshot;

/// The sent stroke waiting for the core's answer, if any.
#[derive(Debug, Default)]
pub(crate) struct CommitSequence {
    // The commit's request id, 0 while none waits.
    request_id: i64,
    // The stroke as sent (the border check reads its ends); the buffer is reused.
    points: Vec<Vector3>,
    // The graded preview epoch the commit adopted: its tiles show until the commit's edit
    // installs, or until the commit is refused.
    graded_epoch: i64,
}

/// What the core's answer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitStep {
    /// Accepted: the preview stays until the chunks of `generation` are installed. With
    /// `border_node`, an end reached the map edge: the chain ended there and the border dialog
    /// asks about that node.
    Committed {
        /// The render generation whose road chunks replace the preview's.
        generation: u64,
        /// The border connection candidate at an end of the stroke.
        border_node: Option<i64>,
    },
    /// Refused: the graded renderer drops the tiles of `graded_epoch`. `rejected`: the stroke
    /// is editable again and shows the refusal (not after a cancel while it waited).
    Refused {
        /// The graded preview epoch the commit had adopted.
        graded_epoch: i64,
        /// The stroke stays on screen with its refusal.
        rejected: bool,
    },
}

impl CommitSequence {
    /// A confirming click on the stroke the session sent (`take_commit`), decided by `outcome`,
    /// the preview of exactly that stroke: true when it may be sent (the caller then sends it
    /// and calls [`Self::sent`]); a refused preview is the session's refusal at once, the stroke
    /// staying editable. `O(1)`.
    pub(crate) fn click(
        session: &mut RoadToolSession,
        view: &RoadToolQuerySnapshot,
        outcome: &PreviewOutcome,
    ) -> bool {
        if !outcome.valid {
            session.handle_event(view, RoadToolEvent::CommitAck { committed: false });
        }
        outcome.valid
    }

    /// The click's stroke was queued as `request_id` (`points` as sent), adopting the graded
    /// preview tiles of `graded_epoch`. `O(points)`, no allocation once the buffer has grown.
    pub(crate) fn sent(&mut self, request_id: i64, points: &[Vector3], graded_epoch: i64) {
        self.request_id = request_id;
        self.points.clear();
        self.points.extend_from_slice(points);
        self.graded_epoch = graded_epoch;
    }

    /// The commit waiting for its answer, 0 for none.
    pub(crate) fn request_id(&self) -> i64 {
        self.request_id
    }

    /// The waiting commit's stroke, empty for none.
    pub(crate) fn points(&self) -> &[Vector3] {
        if self.request_id == 0 {
            &[]
        } else {
            &self.points
        }
    }

    /// The graded preview epoch the waiting commit adopted, -1 for none.
    pub(crate) fn graded_epoch(&self) -> i64 {
        if self.request_id == 0 {
            -1
        } else {
            self.graded_epoch
        }
    }

    /// The core's answer to the waiting commit: the session hears it (an acceptance with a
    /// `border_node` at an end also cancels, ending the chain), a refusal turns `standing`, the
    /// standing preview, into the commit's rejection. `border_node` is read only for an
    /// acceptance. `O(1)` beyond the session's event.
    pub(crate) fn answer(
        &mut self,
        session: &mut RoadToolSession,
        view: &RoadToolQuerySnapshot,
        committed: bool,
        generation: u64,
        border_node: Option<i64>,
        standing: Option<&mut PreviewOutcome>,
    ) -> CommitStep {
        let graded_epoch = self.graded_epoch();
        self.request_id = 0;
        session.handle_event(view, RoadToolEvent::CommitAck { committed });
        if committed {
            if border_node.is_some() {
                session.handle_event(view, RoadToolEvent::Action(RoadToolAction::Cancel));
            }
            return CommitStep::Committed {
                generation,
                border_node,
            };
        }
        if let Some(standing) = standing {
            standing.reject();
        }
        let phase = session.frame().state.phase;
        CommitStep::Refused {
            graded_epoch,
            rejected: matches!(phase, Phase::Placing { .. } | Phase::Editing),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::outcome::COMMIT_REJECTED_REASON;
    use super::super::session::{Pointer, PointerButton};
    use super::super::tool_tests::view;
    use super::*;

    // A vertical camera ray onto `(x, z)`.
    fn ray(x: f32, z: f32) -> Pointer {
        Pointer::Ray {
            origin: Vector3::new(x, 300.0, z),
            dir: Vector3::new(0.0, -1.0, 0.0),
        }
    }

    fn move_to(session: &mut RoadToolSession, x: f32, z: f32) {
        let pointer = ray(x, z);
        session.handle_event(
            view(),
            RoadToolEvent::Pointer {
                pointer,
                shift: false,
                ctrl: false,
            },
        );
    }

    fn click_at(session: &mut RoadToolSession, x: f32, z: f32) {
        move_to(session, x, z);
        session.handle_event(view(), RoadToolEvent::Press(PointerButton::Left));
    }

    fn valid() -> PreviewOutcome {
        PreviewOutcome {
            pose: 3,
            valid: true,
            build_cost: 250.0,
            ..PreviewOutcome::default()
        }
    }

    // Two clicks on open ground send a stroke; a valid preview queues it as request 9 at epoch 4.
    fn sent_stroke(
        session: &mut RoadToolSession,
        commit: &mut CommitSequence,
        end: (f32, f32),
    ) -> Vec<Vector3> {
        click_at(session, -200.0, 150.0);
        click_at(session, end.0, end.1);
        let stroke = session
            .take_commit()
            .expect("the second click sends the stroke");
        assert!(CommitSequence::click(session, view(), &valid()));
        commit.sent(9, &stroke.points, 4);
        assert_eq!(session.frame().state.phase, Phase::AwaitingAck);
        assert_eq!((commit.request_id(), commit.graded_epoch()), (9, 4));
        assert_eq!(commit.points(), stroke.points.as_slice());
        stroke.points
    }

    // Accepted: the preview holds for the generation, the chain continues at the stroke's end,
    // nothing waits any more.
    #[test]
    fn an_accepted_commit_chains_and_clears() {
        let (mut session, mut commit) = (RoadToolSession::new(), CommitSequence::default());
        let points = sent_stroke(&mut session, &mut commit, (-150.0, 260.0));
        let mut standing = valid();
        let step = commit.answer(&mut session, view(), true, 12, None, Some(&mut standing));
        assert_eq!(
            step,
            CommitStep::Committed {
                generation: 12,
                border_node: None
            }
        );
        assert_eq!(session.frame().state.phase, Phase::Placing { placed: 1 });
        move_to(&mut session, -100.0, 300.0);
        assert_eq!(session.frame().stroke.points[0], points[1]);
        assert!(
            standing.valid,
            "an acceptance leaves the standing preview alone"
        );
        assert_eq!((commit.request_id(), commit.graded_epoch()), (0, -1));
        assert!(commit.points().is_empty());
    }

    // Refused: the stroke stays editable, the standing preview becomes the rejection, the
    // renderer drops the commit's graded epoch.
    #[test]
    fn a_refused_commit_rejects_the_standing_preview() {
        let (mut session, mut commit) = (RoadToolSession::new(), CommitSequence::default());
        let points = sent_stroke(&mut session, &mut commit, (-150.0, 260.0));
        let mut standing = valid();
        let step = commit.answer(&mut session, view(), false, 0, Some(5), Some(&mut standing));
        assert_eq!(
            step,
            CommitStep::Refused {
                graded_epoch: 4,
                rejected: true
            }
        );
        assert_eq!(session.frame().state.phase, Phase::Placing { placed: 1 });
        assert_eq!(session.frame().stroke.points[0], points[0]);
        assert!(!standing.valid && !standing.pending);
        assert_eq!(standing.reason, COMMIT_REJECTED_REASON);
        assert_eq!(commit.request_id(), 0);
    }

    // Refused, then the unchanged stroke clicked again on the same context: its standing
    // preview is the rejection, so the click is refused at once and nothing is sent.
    #[test]
    fn a_refused_stroke_clicked_unchanged_is_refused_again() {
        let (mut session, mut commit) = (RoadToolSession::new(), CommitSequence::default());
        let points = sent_stroke(&mut session, &mut commit, (-150.0, 260.0));
        let mut standing = valid();
        commit.answer(&mut session, view(), false, 0, None, Some(&mut standing));
        click_at(&mut session, -150.0, 260.0);
        let again = session
            .take_commit()
            .expect("the click sends the stroke again");
        assert_eq!(again.points, points);
        assert!(!CommitSequence::click(&mut session, view(), &standing));
        assert_eq!(session.frame().state.phase, Phase::Placing { placed: 1 });
        assert_eq!(commit.request_id(), 0, "nothing was sent");
    }

    // Accepted with a border candidate at an end: the border node is asked about and the chain
    // ends there, though the stroke would chain without it.
    #[test]
    fn a_border_end_ends_the_chain() {
        for border_node in [Some(17), None] {
            let (mut session, mut commit) = (RoadToolSession::new(), CommitSequence::default());
            sent_stroke(&mut session, &mut commit, (-150.0, 260.0));
            let step = commit.answer(&mut session, view(), true, 3, border_node, None);
            assert_eq!(
                step,
                CommitStep::Committed {
                    generation: 3,
                    border_node
                }
            );
            let expected = match border_node {
                Some(_) => Phase::Idle,
                None => Phase::Placing { placed: 1 },
            };
            assert_eq!(
                session.frame().state.phase,
                expected,
                "border {border_node:?}"
            );
        }
    }

    // Cancel while pending: an acceptance lands without a chain; a refusal does not bring the
    // stroke back and shows no rejection.
    #[test]
    fn cancel_while_pending() {
        for committed in [true, false] {
            let (mut session, mut commit) = (RoadToolSession::new(), CommitSequence::default());
            sent_stroke(&mut session, &mut commit, (-150.0, 260.0));
            session.handle_event(view(), RoadToolEvent::Action(RoadToolAction::Cancel));
            assert_eq!(commit.request_id(), 9, "the sent stroke still waits");
            let step = commit.answer(&mut session, view(), committed, 6, None, None);
            let expected = if committed {
                CommitStep::Committed {
                    generation: 6,
                    border_node: None,
                }
            } else {
                CommitStep::Refused {
                    graded_epoch: 4,
                    rejected: false,
                }
            };
            assert_eq!(step, expected, "committed {committed}");
            assert_eq!(session.frame().state.phase, Phase::Idle);
        }
    }
}
