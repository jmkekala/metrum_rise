// SPDX-License-Identifier: GPL-3.0-or-later

//! How walkers on one walkway make room for each other. A walkway is two foot lanes on one
//! centre line, one per direction ([`LaneSystem::opposing_foot_lane`]); each lane's walkers sit
//! in its sorted lane bucket, so a walker's neighbours are a binary search away in its own
//! bucket and its opposing lane's.
//!
//! The simulation decides who passes: a faster walker overtakes a slower one ahead unless the
//! walkway beside the slower one is taken, by a walker coming the other way or by one already
//! walking abreast of it, and then follows at the slower one's pace ([`walker_lane_speed`]).
//! Walkers queue only behind one standing still, bodies the minimum gap apart (the lane
//! buckets' overlap correction); moving walkers pass through each other's lane distance.
//!
//! The render snapshot draws who makes room where ([`walker_lateral_m`]): a walker meeting one
//! coming the other way keeps to its side, a walker overtaking takes the other side and the
//! one overtaken keeps to its side. Each is a pure function of the walkers' lane distances and
//! paces, so the drawn offsets follow the walkers smoothly with no stored state.
//!
//! Both read the tick's lane buckets, never another walker's live state, so they are
//! deterministic under any worker count.
//!
//! [`LaneSystem::opposing_foot_lane`]: crate::simulation::network::lanes::LaneSystem::opposing_foot_lane

use super::LaneEntry;
use super::walker_gait::SIDE_OFFSET_MAX_M;
use crate::config::{AGENT_WALK_SPEED_MS, DRIVE_ON_LEFT};

/// A walker standing still moves slower than this, m/s.
pub(crate) const WALKER_STANDING_MS: f32 = 0.05;

// A faster walker this close behind a slower one decides to pass it or follow it, metres.
const PASS_LOOK_M: f32 = 3.0;
// An overtake needs the walkway clear of oncoming walkers from this far behind the overtaker to
// this far ahead of the walker overtaken, metres.
const PASS_CLEAR_BEHIND_M: f32 = 1.0;
const PASS_CLEAR_AHEAD_M: f32 = 4.0;
// Two walkers this close along the lane walk side by side and fill the walkway, metres.
const ABREAST_M: f32 = 0.8;

// How far to its side a walker making room keeps from the centre line, metres.
const KEEP_SIDE_M: f32 = 0.42;
// Same-lane walkers start making room at this distance along the lane, fully by the near one:
// people drift aside well before they are level, not in the last step.
const PASS_SIDE_FAR_M: f32 = 5.0;
const PASS_SIDE_NEAR_M: f32 = 1.5;
// An oncoming walker this far ahead starts a walker keeping to its side, fully by the near
// distance. After they meet, the walker holds its side for the hold distance, then drifts back
// by the behind distance.
const ONCOMING_FAR_M: f32 = 10.0;
const ONCOMING_NEAR_M: f32 = 4.0;
const ONCOMING_HOLD_M: f32 = 1.0;
const ONCOMING_BEHIND_M: f32 = 6.0;
// Neighbours read on each side: more cannot change the offset in a walkway's width.
const SIDE_NEIGHBOURS: usize = 3;

/// A lane bucket: `(lane distance, agent, half length)` sorted by distance, then agent.
pub(crate) type Bucket<'a> = &'a [LaneEntry];

/// The opposing lane of a walker's walkway: its bucket and length.
#[derive(Clone, Copy)]
pub(crate) struct Opposing<'a> {
    /// The opposing lane's bucket.
    pub bucket: Bucket<'a>,
    /// The opposing lane's length, metres.
    pub length: f32,
}

// First entry of `bucket` ordered after `(d, agent)`. O(log bucket).
fn after(bucket: Bucket<'_>, d: f32, agent: usize) -> usize {
    bucket.partition_point(|&(ed, ea, _)| ed < d || (ed == d && ea <= agent))
}

// First entry of `bucket` ordered at or after `(d, agent)`. O(log bucket).
fn at(bucket: Bucket<'_>, d: f32, agent: usize) -> usize {
    bucket.partition_point(|&(ed, ea, _)| ed < d || (ed == d && ea < agent))
}

impl Opposing<'_> {
    // The opposing walkers along a lane of `length`, from `lo` to `hi` on it, nearest `lo` first:
    // each one's distance on that lane and agent. The lanes share their centre line, so a distance
    // maps by its fraction.
    fn within(self, length: f32, lo: f32, hi: f32) -> impl Iterator<Item = (f32, usize)> {
        let scale = self.length / length.max(f32::EPSILON);
        let (from, to) = (self.length - hi * scale, self.length - lo * scale);
        let end = self.bucket.partition_point(|&(d, _, _)| d <= to);
        self.bucket[..end]
            .iter()
            .rev()
            .take_while(move |&&(d, _, _)| d >= from)
            .map(move |&(d, agent, _)| ((self.length - d) / scale, agent))
    }
}

/// The speed of walker `agent` at `d` on a foot lane of `length`, m/s: `own`, its pace, or the
/// pace of the slower walker just ahead when the walkway beside that walker is taken. `pace_of`
/// gives a walker's pace speed; `opposing` is asked only with a slower walker close ahead.
/// O(log bucket).
#[allow(clippy::too_many_arguments)]
pub(crate) fn walker_lane_speed<'a>(
    own: f32,
    d: f32,
    agent: usize,
    length: f32,
    bucket: Bucket<'_>,
    opposing: impl FnOnce() -> Option<Opposing<'a>>,
    pace_of: impl Fn(usize) -> f32,
) -> f32 {
    let ahead = &bucket[after(bucket, d, agent)..];
    let Some(&(lead_d, lead, _)) = ahead.first() else {
        return own;
    };
    if lead_d - d > PASS_LOOK_M {
        return own;
    }
    let lead_speed = pace_of(lead);
    if lead_speed >= own {
        return own;
    }
    let abreast = ahead
        .get(1)
        .is_some_and(|&(beside, _, _)| beside - lead_d < ABREAST_M);
    let oncoming = !abreast
        && opposing().is_some_and(|opposing| {
            let (lo, hi) = (d - PASS_CLEAR_BEHIND_M, lead_d + PASS_CLEAR_AHEAD_M);
            opposing.within(length, lo, hi).next().is_some()
        });
    if abreast || oncoming { lead_speed } else { own }
}

/// The drawn distance of walker `agent` at `d` from its foot lane's centre line, metres, left of
/// travel positive, within [`SIDE_OFFSET_MAX_M`]: its own side `preferred` on an empty walkway,
/// its keep side while it meets an oncoming walker or is overtaken, and the other side while it
/// overtakes. `pace_of` gives a walker's pace. With `ahead_s` above zero, the side `ahead_s`
/// seconds on, everyone moved on at `speed_of` (m/s): the drawn body turns toward it. O(log
/// bucket).
#[allow(clippy::too_many_arguments)]
pub(crate) fn walker_lateral_m(
    preferred: f32,
    d: f32,
    agent: usize,
    pace: f32,
    length: f32,
    bucket: Bucket<'_>,
    opposing: Option<Opposing<'_>>,
    pace_of: impl Fn(usize) -> f32,
    ahead_s: f32,
    speed_of: impl Fn(usize) -> f32,
) -> f32 {
    // Pedestrians keep to the side traffic drives on.
    let keep = if DRIVE_ON_LEFT {
        KEEP_SIDE_M
    } else {
        -KEEP_SIDE_M
    };
    let (mut keep_w, mut pass_w) = (0.0_f32, 0.0_f32);
    // Where a walker is `ahead_s` on along this lane; an opposing one walks toward lower `d`.
    let moved = |other_d: f32, other: usize| other_d + speed_of(other) * ahead_s;
    let own_d = moved(d, agent);
    let behind = bucket[..at(bucket, d, agent)].iter().rev();
    let ahead = bucket[after(bucket, d, agent)..].iter();
    for &(other_d, other, _) in behind
        .take(SIDE_NEIGHBOURS)
        .chain(ahead.take(SIDE_NEIGHBOURS))
    {
        let apart = (moved(other_d, other) - own_d).abs();
        let w = 1.0 - smoothstep(PASS_SIDE_NEAR_M, PASS_SIDE_FAR_M, apart);
        // The faster walker of a pair takes the passing side; equal paces split by agent.
        let other_pace = pace_of(other);
        if pace > other_pace || (pace == other_pace && agent < other) {
            pass_w = pass_w.max(w);
        } else {
            keep_w = keep_w.max(w);
        }
    }
    if let Some(opposing) = opposing {
        // Window widened by how far a walker at the mean speed goes in `ahead_s`.
        let reach = AGENT_WALK_SPEED_MS * ahead_s;
        let near = opposing
            .within(
                length,
                d - ONCOMING_BEHIND_M - reach,
                d + ONCOMING_FAR_M + reach,
            )
            .take(SIDE_NEIGHBOURS);
        for (other_d, other) in near {
            let ahead_m = other_d - speed_of(other) * ahead_s - own_d;
            let w = if ahead_m >= 0.0 {
                1.0 - smoothstep(ONCOMING_NEAR_M, ONCOMING_FAR_M, ahead_m)
            } else {
                1.0 - smoothstep(ONCOMING_HOLD_M, ONCOMING_BEHIND_M, -ahead_m)
            };
            keep_w = keep_w.max(w);
        }
    }
    let side = preferred + (-keep - preferred) * pass_w;
    let side = side + (keep - side) * keep_w;
    side.clamp(-SIDE_OFFSET_MAX_M, SIDE_OFFSET_MAX_M)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEN: f32 = 100.0;
    const FAST: f32 = 1.6;
    const SLOW: f32 = 1.2;
    // A walker's half body length, as its lane bucket entry holds it.
    const HALF: f32 = crate::config::WALKER_BODY_LENGTH_M * 0.5;
    // Agents 0..: agent 0 fast, others slow.
    fn pace(agent: usize) -> f32 {
        if agent == 0 { FAST } else { SLOW }
    }
    fn opposing(bucket: Bucket<'_>) -> Option<Opposing<'_>> {
        Some(Opposing {
            bucket,
            length: LEN,
        })
    }
    // A walker at `d` on the opposing lane, as that lane measures it.
    fn facing(d: f32, agent: usize) -> LaneEntry {
        (LEN - d, agent, HALF)
    }

    #[test]
    fn a_faster_walker_passes_on_a_clear_walkway_and_follows_when_it_is_taken() {
        let lane = [(10.0, 0, HALF), (12.0, 1, HALF)];
        let speed = |opp: &[LaneEntry]| {
            walker_lane_speed(FAST, 10.0, 0, LEN, &lane, || opposing(opp), pace)
        };
        assert_eq!(speed(&[]), FAST, "clear walkway: overtakes");
        assert_eq!(
            speed(&[facing(15.0, 9)]),
            SLOW,
            "oncoming in the pass: follows"
        );
        assert_eq!(
            speed(&[facing(30.0, 9)]),
            FAST,
            "oncoming far ahead: overtakes"
        );
        assert_eq!(
            speed(&[facing(5.0, 9)]),
            FAST,
            "oncoming already past: overtakes"
        );

        let oncoming = [facing(15.0, 9)];
        let pair = [(10.0, 0, HALF), (12.0, 1, HALF), (12.5, 2, HALF)];
        let abreast = walker_lane_speed(FAST, 10.0, 0, LEN, &pair, || None, pace);
        assert_eq!(abreast, SLOW, "two abreast ahead: follows");
        let far = [(10.0, 0, HALF), (14.0, 1, HALF)];
        assert_eq!(
            walker_lane_speed(FAST, 10.0, 0, LEN, &far, || None, pace),
            FAST
        );
        let slower_behind = [(8.0, 1, HALF), (10.0, 0, HALF)];
        assert_eq!(
            walker_lane_speed(FAST, 10.0, 0, LEN, &slower_behind, || None, pace),
            FAST
        );
        assert_eq!(
            walker_lane_speed(SLOW, 12.0, 1, LEN, &lane, || opposing(&oncoming), pace),
            SLOW,
            "a walker with no slower one ahead keeps its pace"
        );
    }

    #[test]
    fn walkers_keep_to_their_side_for_oncoming_walkers_and_pass_on_the_other() {
        let keep = if DRIVE_ON_LEFT {
            KEEP_SIDE_M
        } else {
            -KEEP_SIDE_M
        };
        let side = |d: f32, agent: usize, lane: &[LaneEntry], opp: &[LaneEntry]| {
            let still = |_| 0.0;
            walker_lateral_m(
                0.1,
                d,
                agent,
                pace(agent),
                LEN,
                lane,
                opposing(opp),
                pace,
                0.0,
                still,
            )
        };
        let alone = [(10.0, 1, HALF)];
        assert_eq!(side(10.0, 1, &alone, &[]), 0.1, "alone: its own side");
        assert!((side(10.0, 1, &alone, &[facing(11.0, 9)]) - keep).abs() < 1e-6);
        assert_eq!(
            side(10.0, 1, &alone, &[facing(25.0, 9)]),
            0.1,
            "far: not yet"
        );
        let easing = side(10.0, 1, &alone, &[facing(17.0, 9)]);
        assert!(easing < 0.1 && easing > keep || easing > 0.1 && easing < keep);

        let pair = [(10.0, 0, HALF), (10.5, 1, HALF)];
        let passer = side(10.0, 0, &pair, &[]);
        let passed = side(10.5, 1, &pair, &[]);
        assert!(
            (passer + keep).abs() < 1e-6,
            "the faster walker takes the other side"
        );
        assert!(
            (passed - keep).abs() < 1e-6,
            "the slower walker keeps to its side"
        );
        assert!((passer - passed).abs() > 0.8, "side by side, apart");
    }

    #[test]
    fn a_walker_meeting_someone_drifts_aside_early_and_back_late() {
        let keep = if DRIVE_ON_LEFT {
            KEEP_SIDE_M
        } else {
            -KEEP_SIDE_M
        };
        let walking = |_| 1.4;
        let side = |ahead: f32, ahead_s: f32| {
            let opp = [facing(10.0 + ahead, 9)];
            let lane = [(10.0, 1, HALF)];
            let pace = pace(1);
            let opposing = opposing(&opp);
            walker_lateral_m(
                0.1,
                10.0,
                1,
                pace,
                LEN,
                &lane,
                opposing,
                super::tests::pace,
                ahead_s,
                walking,
            )
        };
        // Coming 8 m ahead: aside already, and further aside half a second on.
        let (now, soon) = (side(8.0, 0.0), side(8.0, 0.5));
        assert!(
            now != 0.1 && (soon - keep).abs() < (now - keep).abs(),
            "{now} then {soon}"
        );
        // Just met: still aside; well past: back on its own side.
        assert!((side(-0.5, 0.0) - keep).abs() < 1e-6);
        assert_eq!(side(-8.0, 0.0), 0.1);
    }

    #[test]
    fn the_drawn_side_moves_smoothly_as_walkers_close_in() {
        let mut last = None;
        for step in 0..=400 {
            let d = 10.0 + step as f32 * 0.02;
            let lane = [(d, 0, HALF), (14.0, 1, HALF)];
            let s = walker_lateral_m(
                0.1,
                d,
                0,
                FAST,
                LEN,
                &lane,
                opposing(&[facing(16.0, 9)]),
                pace,
                0.0,
                |_| 0.0,
            );
            assert!(s.abs() <= SIDE_OFFSET_MAX_M);
            if let Some(prev) = last {
                let step_m: f32 = s - prev;
                assert!(step_m.abs() < 0.03, "jump of {step_m} m at d {d}");
            }
            last = Some(s);
        }
    }
}
