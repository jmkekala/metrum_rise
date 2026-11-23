// SPDX-License-Identifier: GPL-3.0-or-later

//! The leaving line at an access mouth (`PARK-32`): one vehicle at a time stands at the mouth of a
//! lot, driveway, kerbside place or yard gate; the others wait behind it on their way out.
//!
//! A pull-out's polyline runs from its place along its way out to the mouth (the polyline's last
//! point but one, where the way meets the road, shared by every place of one gate) and on to its
//! lane point. Every vehicle pulling out through one mouth, and a car held at its lane point
//! after its pull-out (`hold_for_lane_attach!`), is one entry of that mouth's line, measured by
//! `m`, its distance along its own polyline to the mouth (negative past it). In the order they
//! stand (`m`, then the time each asked to leave, `journey_start_time`, then agent handle) each
//! holds a body and 1 m short of the one ahead: half of each body and 1 m
//! (`EXIT_LINE_GAP_M`), along its own way. A vehicle still at its place (the polyline's first
//! point) leaves it only when every vehicle already under way through the mouth is that far
//! ahead of it, and then only the first of those waiting at the mouth's places in the order they
//! asked to leave; meanwhile it stays where it was parked. The result is the least distance to
//! its polyline's end each may reach in the next movement pass, read by the egress step
//! (`ParkingWork::exit_holds`).
//!
//! Scratch only, nothing saved: the members are kept from the commits that start pull-outs and
//! from a dock's start, checked against the rows before each movement pass, and found by a scan
//! of the rows on a load or undo; the holds are a function of the rows. O(L log L + L × way
//! points) a tick for L vehicles in lines, no allocation past the largest tick.

use super::super::super::{
    ACCESS_LANE_ATTACH_PENDING, AgentHandle, AgentSystem, TRANSIT_ACCESS_EGRESS, TRANSIT_NETWORK,
};
use super::super::access::left_along;
use super::super::traffic::lane_half_length;
use crate::simulation::network::TransitNetwork;
use crate::simulation::parking::{ParkingSystem, ParkingTrip, ParkingTripPhase};
use godot::prelude::Vector2;

// What a vehicle in a leaving line keeps clear ahead of its body: the gap between two bodies.
const EXIT_LINE_GAP_M: f32 = 1.0;

/// One vehicle of a leaving line, rebuilt every tick.
#[derive(Clone, Copy)]
pub(in super::super) struct ExitEntry {
    // The mouth's position bits: the line's key.
    mouth: u64,
    // Its distance along its own polyline to the mouth; negative past it.
    m: f32,
    // Still at its place: not yet under way.
    parked: bool,
    // The time it asked to leave, then its handle: the order of those still parked.
    asked: f32,
    agent: AgentHandle,
    index: usize,
    pos: Vector2,
    half: f32,
    // Its polyline's last leg, mouth to lane point, and its whole length.
    hop: f32,
    length: f32,
}

// Whether agent row `trip` / `transit` / `lane` / `flags` stands in a leaving line: pulling out of
// a place or dock, or a resident's car held at the lane point of the pull-out it has just
// finished. A drop-off leaving its kerb stop (no place) and a truck already off its dock's way are
// in none. O(1).
pub(super) fn in_exit_line(trip: &ParkingTrip, transit: u8, lane: usize, flags: u8) -> bool {
    if trip.source.is_none() {
        return false;
    }
    match trip.phase {
        ParkingTripPhase::PullOut => transit == TRANSIT_ACCESS_EGRESS,
        ParkingTripPhase::Drive => {
            !trip.freight_gate
                && !trip.drop_off
                && transit == TRANSIT_NETWORK
                && lane == usize::MAX
                && flags & ACCESS_LANE_ATTACH_PENDING != 0
        }
        _ => false,
    }
}

impl AgentSystem {
    /// Keeps agent `i` among the leaving lines' members if its row stands in one: after a commit
    /// visit or a dock start may have started its pull-out. O(1).
    pub(in crate::simulation::economy::agents) fn note_exit_line_member(&mut self, i: usize) {
        let a = &self.agents;
        if in_exit_line(
            &a.parking_trip[i],
            a.transit[i],
            a.current_lane_id[i],
            a.access_flags[i],
        ) {
            self.parking_work.exit_members.push(self.handles.handle(i));
        }
    }

    /// The leaving lines before a movement pass: members checked against their rows, measured,
    /// ordered, and each one's hold for the pass (`ParkingWork::exit_holds`). O(L log L + L ×
    /// way points).
    pub(in super::super) fn keep_exit_lines(
        &mut self,
        parking: &ParkingSystem,
        network: &TransitNetwork,
    ) {
        let mut members = std::mem::take(&mut self.parking_work.exit_members);
        let mut entries = std::mem::take(&mut self.parking_work.exit_entries);
        let mut holds = std::mem::take(&mut self.parking_work.exit_holds);
        entries.clear();
        holds.clear();
        members.sort_unstable_by_key(|agent| agent.to_bits());
        members.dedup();
        members.retain(|&agent| {
            let Some(i) = self.handles.resolve(agent) else {
                return false;
            };
            let a = &self.agents;
            let trip = a.parking_trip[i];
            if !in_exit_line(&trip, a.transit[i], a.current_lane_id[i], a.access_flags[i]) {
                return false;
            }
            let lane = a.planned_attach_lane_id[i] as usize;
            let Some(path) = parking.maneuver_path(
                trip.source,
                lane,
                a.planned_attach_lane_d[i],
                network,
                false,
                trip.freight_gate,
            ) else {
                return true;
            };
            let path = path.as_slice();
            let n = path.len();
            if n < 2 {
                return true;
            }
            let pos = Vector2::new(a.pos_x[i], a.pos_y[i]);
            let left = left_along(path, pos);
            // Off its own pull-out (a held car that left it long ago): in no line.
            if !left.is_finite() {
                return true;
            }
            let hop = path[n - 2].distance_to(path[n - 1]);
            let length: f32 = path.windows(2).map(|w| w[0].distance_to(w[1])).sum();
            let mouth = path[n - 2];
            entries.push(ExitEntry {
                mouth: (u64::from(mouth.x.to_bits()) << 32) | u64::from(mouth.y.to_bits()),
                m: left - hop,
                parked: trip.phase == ParkingTripPhase::PullOut && left >= length - 0.01,
                asked: a.journey_start_time[i],
                agent,
                index: i,
                pos,
                half: lane_half_length(a.transit_mode[i], a.vehicle_type[i]),
                hop,
                length,
            });
            true
        });
        // By mouth; under way before parked; under way by where they stand, parked by when they
        // asked to leave; then by handle.
        entries.sort_unstable_by(|x, y| {
            x.mouth
                .cmp(&y.mouth)
                .then(x.parked.cmp(&y.parked))
                .then_with(|| {
                    if x.parked {
                        x.asked.total_cmp(&y.asked)
                    } else {
                        x.m.total_cmp(&y.m).then(x.asked.total_cmp(&y.asked))
                    }
                })
                .then(x.agent.to_bits().cmp(&y.agent.to_bits()))
        });
        let mut k = 0;
        while k < entries.len() {
            let end = k + entries[k..]
                .iter()
                .take_while(|e| e.mouth == entries[k].mouth)
                .count();
            hold_line(&entries[k..end], &mut holds);
            k = end;
        }
        holds.sort_unstable_by_key(|&(i, _)| i);
        self.parking_work.exit_members = members;
        self.parking_work.exit_entries = entries;
        self.parking_work.exit_holds = holds;
    }
}

// One mouth's line, under way first in the order they stand, then the parked in the order they
// asked to leave: each one's hold, the least distance to its polyline's end it may reach, where it
// is not zero. O(line).
fn hold_line(line: &[ExitEntry], holds: &mut Vec<(usize, f32)>) {
    let mut ahead: Option<&ExitEntry> = None;
    let mut first_parked = true;
    for e in line {
        let behind = |a: &ExitEntry| e.hop + a.m + a.half + e.half + EXIT_LINE_GAP_M;
        if !e.parked {
            if let Some(a) = ahead {
                holds.push((e.index, behind(a).max(0.0)));
            }
            ahead = Some(e);
            continue;
        }
        // A parked one leaves its place only first in order and with the last one under way a
        // body and 1 m ahead of where it stands, along the way and in a straight line (beside
        // its place the two ways may run side by side); else it stays, its hold its whole way.
        let room = ahead.is_none_or(|a| {
            let gap = a.half + e.half + EXIT_LINE_GAP_M;
            a.m + gap <= e.m && a.pos.distance_to(e.pos) >= gap
        });
        let hold = match ahead {
            Some(a) if first_parked && room => behind(a).max(0.0),
            None if first_parked => 0.0,
            _ => e.length,
        };
        first_parked = false;
        if hold > 0.0 {
            holds.push((e.index, hold));
        }
    }
}

/// The hold of agent `i` in `holds` (sorted by agent index), if it waits in a leaving line: the
/// least distance to the end of its pull-out it may reach this pass. O(log holds).
pub(in super::super) fn exit_hold(holds: &[(usize, f32)], i: usize) -> Option<f32> {
    holds
        .binary_search_by_key(&i, |&(j, _)| j)
        .ok()
        .map(|k| holds[k].1)
}

#[cfg(test)]
mod tests {
    use super::{ExitEntry, hold_line};
    use crate::simulation::economy::agents::AgentHandle;
    use godot::prelude::Vector2;

    // A car of one mouth's line: `m` from the mouth, a 3 m hop, its whole way `length`.
    fn car(index: usize, m: f32, parked: bool, length: f32) -> ExitEntry {
        ExitEntry {
            mouth: 1,
            m,
            parked,
            asked: index as f32,
            agent: AgentHandle::NONE,
            index,
            pos: Vector2::new(m, 0.0),
            half: 2.25,
            hop: 3.0,
            length,
        }
    }

    #[test]
    fn each_holds_a_body_and_a_metre_behind_the_one_ahead() {
        let mut holds = Vec::new();
        // The head waits at the mouth; the next is 4 m behind it, the third 20 m.
        hold_line(
            &[
                car(0, 0.0, false, 30.0),
                car(1, 4.0, false, 30.0),
                car(2, 20.0, false, 30.0),
            ],
            &mut holds,
        );
        // Neither may come nearer its end than its hop, 5.5 m past the one ahead.
        assert_eq!(holds, vec![(1, 8.5), (2, 12.5)]);
    }

    // A single place whose way out is shorter than a body and 1 m (a short driveway): with a
    // car standing at its mouth it stays parked, its hold its whole way; with the mouth clear it
    // leaves.
    #[test]
    fn a_place_with_no_room_behind_the_line_keeps_its_car() {
        let mut holds = Vec::new();
        hold_line(
            &[car(0, 0.0, false, 30.0), car(1, 4.0, true, 7.0)],
            &mut holds,
        );
        assert_eq!(holds, vec![(1, 7.0)]);
        holds.clear();
        hold_line(&[car(1, 4.0, true, 7.0)], &mut holds);
        assert!(holds.is_empty());
        // Two parked: only the first to ask leaves, with room.
        holds.clear();
        hold_line(
            &[
                car(0, -3.0, false, 30.0),
                car(1, 9.0, true, 12.0),
                car(2, 25.0, true, 28.0),
            ],
            &mut holds,
        );
        assert_eq!(holds, vec![(1, 5.5), (2, 28.0)]);
    }
}
