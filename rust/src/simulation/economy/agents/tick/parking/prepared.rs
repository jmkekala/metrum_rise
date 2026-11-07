// SPDX-License-Identifier: GPL-2.0-only

//! The routing of a tick's parking commit, worked out in parallel before it starts.
//!
//! The commit in `parking.rs` walks its candidates one at a time in ascending agent order,
//! because a visit can take or free a bay the next visit wants. Most of a visit's cost is
//! routing, though, and routing reads nothing the commit writes. During a commit:
//!
//! - lanes, roads, the routing hierarchy and the building entrances are fixed;
//! - an agent's row changes only in its own visit, and each agent is visited once;
//! - a bay that is active stays, with its access and its geometry: the commit only ever removes
//!   bays already inactive (`clear_retired_places`).
//!
//! So a leg's quote and route are pure functions of its two ends.
//! [`AgentSystem::prepare_parking_answers`] has every worker take candidates off one list and
//! answer, from the state as a block of candidates starts, the legs each visit is expected to ask
//! about. Nothing in this pass decides anything, and a wrong expectation costs only time. The
//! commit still decides everything, in the same order as before: it forms each question from the
//! live state and takes an answer only if it is the answer to that question ([`LegKey`],
//! [`ParkingTripBays`]); otherwise it asks the router itself. That is what happens when an earlier
//! visit moved the first free bay at a destination, the one input of a quote the commit changes.
//!
//! Answers are the same at any worker count and from any worker's scratch: a scratch memoizes
//! only exact results, and so does the phase's [`SharedNetworkQuotes`], which keeps network legs
//! from tick to tick so a commute priced yesterday costs no search today, whichever worker meets
//! it. Debug test builds ask the router again, past that cache, for every answer the commit takes
//! and compare the bits. Per tick O(C × (k + Q)) spread over the workers, C candidates, k bays in
//! a catchment and Q a route query, and O(C) serial. No allocation past the busiest tick so far.

use super::super::planning::{
    ParkingLegQuote, ParkingQuote, ParkingTripBays, SharedNetworkQuotes, TripQueryScratch,
    parking_trip_bays, quote_parking_between_places, quote_parking_leg,
};
use super::{
    AGE_CHILD, AgentSystem, MODE_WALK, ParkingTripPhase, next_parking_leg, walk_is_quicker,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::Handle;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::parking::{ParkingSystem, Place};
use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

// Below this a tick's candidates are answered on the calling thread: the fork would cost more
// than the routing it spreads.
const PARALLEL_CANDIDATES: usize = 8;

/// How the visits of a thread's commits were answered since the last [`take_answer_counts`]: the
/// answers they took, and the trips they quoted again because the bays had moved.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::simulation::economy::agents) struct AnswerCounts {
    pub(in crate::simulation::economy::agents) legs: usize,
    pub(in crate::simulation::economy::agents) trips: usize,
    pub(in crate::simulation::economy::agents) routes: usize,
    pub(in crate::simulation::economy::agents) requoted_trips: usize,
}

#[cfg(test)]
thread_local! {
    static COUNTS: std::cell::Cell<AnswerCounts> = const {
        std::cell::Cell::new(AnswerCounts { legs: 0, trips: 0, routes: 0, requoted_trips: 0 })
    };
}

#[cfg(test)]
fn count_answer(count: impl FnOnce(&mut AnswerCounts)) {
    let mut counts = COUNTS.get();
    count(&mut counts);
    COUNTS.set(counts);
}

/// Returns and clears this thread's [`AnswerCounts`].
#[cfg(test)]
pub(in crate::simulation::economy::agents) fn take_answer_counts() -> AnswerCounts {
    COUNTS.take()
}

/// One end of a leg. Within a commit it names exactly one entrance: a door's is fixed, and a live
/// bay's published access does not change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LegEnd {
    Door(usize),
    Bay(Handle<Place>),
}

/// The question a leg's quote answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LegKey {
    pub(super) mode: u8,
    pub(super) from: LegEnd,
    pub(super) to: LegEnd,
}

impl LegKey {
    pub(super) fn on_foot(from: LegEnd, to: LegEnd) -> Self {
        Self {
            mode: MODE_WALK,
            from,
            to,
        }
    }
}

struct PreparedVisit {
    // The leg of a transition, or a departure's walk door to door.
    leg: Option<(LegKey, Option<ParkingLegQuote>)>,
    // A departure's car trip between these bays.
    trip: Option<(ParkingTripBays, Option<ParkingQuote>)>,
    // The nodes of the leg the visit is expected to start, in its shard's `routes`.
    route: Option<(LegKey, Option<(u32, u32)>)>,
}

#[derive(Default)]
struct ShardAnswers {
    // Candidate position and what was prepared for it.
    visits: Vec<(u32, PreparedVisit)>,
    routes: Vec<u32>,
}

/// One tick's answers, reused from tick to tick. Derived: nothing saves them.
#[derive(Default)]
pub(crate) struct ParkingAnswers {
    shards: Vec<ShardAnswers>,
    // Shard and row of each candidate's answers, by the candidate's position in the tick's list.
    index: Vec<Option<(u32, u32)>>,
}

impl ParkingAnswers {
    fn reset(&mut self, shards: usize, candidates: usize) {
        self.shards
            .resize_with(shards.max(self.shards.len()), Default::default);
        for shard in &mut self.shards {
            shard.visits.clear();
            shard.routes.clear();
        }
        self.index.clear();
        self.index.resize(candidates, None);
    }

    fn index_visits(&mut self) {
        for (s, shard) in self.shards.iter().enumerate() {
            for (row, &(k, _)) in shard.visits.iter().enumerate() {
                self.index[k as usize] = Some((s as u32, row as u32));
            }
        }
    }

    /// What was prepared for the candidate at position `k` of the block. O(1).
    pub(super) fn visit<'a>(&'a self, k: usize, quotes: &'a SharedNetworkQuotes) -> KnownVisit<'a> {
        let (visit, routes) = self.index[k].map_or((None, &[][..]), |(shard, row)| {
            let shard = &self.shards[shard as usize];
            (Some(&shard.visits[row as usize].1), &shard.routes[..])
        });
        KnownVisit {
            visit,
            routes,
            quotes,
        }
    }
}

/// The answers a visit may draw on. Each lookup takes the question and returns only the answer
/// to that question.
#[derive(Clone, Copy)]
pub(super) struct KnownVisit<'a> {
    visit: Option<&'a PreparedVisit>,
    routes: &'a [u32],
    quotes: &'a SharedNetworkQuotes,
}

impl KnownVisit<'_> {
    /// The quote for leg `key`, from `ask` unless it was prepared. O(1), or `ask`'s O(Q). `ask`
    /// takes the shared legs it may read.
    pub(super) fn leg_or(
        &self,
        key: LegKey,
        ask: impl Fn(Option<&SharedNetworkQuotes>) -> Option<ParkingLegQuote>,
    ) -> Option<ParkingLegQuote> {
        let prepared = self.visit.and_then(|visit| visit.leg.as_ref());
        let Some((_, quote)) = prepared.filter(|(prepared, _)| *prepared == key) else {
            return ask(Some(self.quotes));
        };
        #[cfg(test)]
        count_answer(|counts| counts.legs += 1);
        #[cfg(all(test, debug_assertions))]
        assert_eq!(
            quote.as_ref().map(ParkingLegQuote::bits),
            ask(None).as_ref().map(ParkingLegQuote::bits),
            "a prepared leg differs from the router's: {key:?}"
        );
        quote.clone()
    }

    /// The quote of the car trip between `bays`, from `ask` unless it was prepared for those
    /// bays. O(1), or `ask`'s O(Q). `ask` takes the shared legs it may read.
    pub(super) fn trip_or(
        &self,
        bays: ParkingTripBays,
        ask: impl Fn(Option<&SharedNetworkQuotes>) -> Option<ParkingQuote>,
    ) -> Option<ParkingQuote> {
        let prepared = self.visit.and_then(|visit| visit.trip.as_ref());
        let Some((_, quote)) = prepared.filter(|(prepared, _)| *prepared == bays) else {
            #[cfg(test)]
            count_answer(|counts| counts.requoted_trips += usize::from(prepared.is_some()));
            return ask(Some(self.quotes));
        };
        #[cfg(test)]
        count_answer(|counts| counts.trips += 1);
        #[cfg(all(test, debug_assertions))]
        assert_eq!(
            quote.as_ref().map(ParkingQuote::bits),
            ask(None).as_ref().map(ParkingQuote::bits),
            "a prepared trip differs from the router's: {bays:?}"
        );
        quote.clone()
    }

    /// The nodes of leg `key`, if they were prepared; inside, `None` for a leg with no route.
    pub(super) fn route(&self, key: LegKey) -> Option<Option<&[u32]>> {
        let prepared = self.visit.and_then(|visit| visit.route.as_ref());
        let (_, nodes) = prepared.filter(|(prepared, _)| *prepared == key)?;
        #[cfg(test)]
        count_answer(|counts| counts.routes += 1);
        Some(nodes.map(|(start, len)| &self.routes[start as usize..(start + len) as usize]))
    }
}

// What was asked and what the router answered, for a leg the visit is expected to start.
fn prepare_route(
    key: LegKey,
    quote: &ParkingLegQuote,
    routes: &mut Vec<u32>,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> (LegKey, Option<(u32, u32)>) {
    let start = routes.len();
    let found = quote.append_route(routes, network, graph);
    let nodes = found.then(|| (start as u32, (routes.len() - start) as u32));
    (key, nodes)
}

impl AgentSystem {
    /// Answers the legs this tick's visits are expected to ask about, from the state at the
    /// phase's start. Reads only; see the module docs for why the results cannot depend on
    /// which worker found them, and the bound. Each worker owns a movement shard's route
    /// scratch, already prepared this tick, and pulls candidates off one counter, so a worker
    /// that wakes late costs the others nothing.
    pub(super) fn prepare_parking_answers(
        &mut self,
        candidates: &[usize],
        quotes: &SharedNetworkQuotes,
        parking: &ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> ParkingAnswers {
        let mut answers = std::mem::take(&mut self.parking_work.answers);
        #[cfg(test)]
        if self.parking_work.answers_off {
            answers.reset(1, candidates.len());
            return answers;
        }
        let mut shards = std::mem::take(&mut self.movement_query_shards);
        let mut serial = std::mem::take(&mut self.movement_serial_query);
        answers.reset(shards.len().max(1), candidates.len());
        let this = &*self;
        let answer = |k: usize, i: usize, shard: &mut ShardAnswers| {
            if let Some(visit) = this.prepare_visit(
                i,
                quotes,
                parking,
                allocator,
                network,
                graph,
                &mut shard.routes,
            ) {
                shard.visits.push((k as u32, visit));
            }
        };
        if shards.is_empty() || candidates.len() < PARALLEL_CANDIDATES {
            let shard = &mut answers.shards[0];
            TripQueryScratch::with_owned(&mut serial, || {
                for (k, &i) in candidates.iter().enumerate() {
                    answer(k, i, shard);
                }
            });
        } else {
            let next = AtomicUsize::new(0);
            shards
                .par_iter_mut()
                .zip(answers.shards.par_iter_mut())
                .for_each(|(scratch, shard)| {
                    TripQueryScratch::with_owned(scratch, || {
                        loop {
                            let k = next.fetch_add(1, Ordering::Relaxed);
                            let Some(&i) = candidates.get(k) else {
                                break;
                            };
                            answer(k, i, shard);
                        }
                    });
                });
        }
        self.movement_query_shards = shards;
        self.movement_serial_query = serial;
        answers.index_visits();
        answers
    }

    /// Prices at load the trips of the departures due within a game minute, so that the first
    /// commits find their network legs as a running game's do. A shift's commuters can all leave
    /// on one minute, and a load cold on that minute pays for every leg at once. Changes no
    /// result: the legs are exact and the answers are dropped. O(agents) to find them, then
    /// O(k + Q) each, spread over the workers.
    pub(crate) fn warm_parking_quotes(
        &mut self,
        time: &crate::simulation::core::time::TimeSystem,
        parking: &ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let a = &self.agents;
        let due: Vec<_> = (0..a.len())
            .into_par_iter()
            .filter(|&i| {
                a.next_departure_day[i] == time.day_index
                    && a.next_departure_minute[i] <= time.minute_of_day + 1
                    && a.next_departure_origin_building[i] == a.current_building[i]
                    && a.next_departure_target_building[i] != usize::MAX
            })
            .collect();
        let quotes = std::mem::take(&mut self.parking_work.quotes);
        let mut shards = std::mem::take(&mut self.movement_query_shards);
        let this = &*self;
        let next = AtomicUsize::new(0);
        shards.par_iter_mut().for_each(|scratch| {
            let mut routes = Vec::new();
            TripQueryScratch::with_owned(scratch, || {
                while let Some(&i) = due.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let (current, target) = (
                        this.agents.current_building[i],
                        this.agents.next_departure_target_building[i],
                    );
                    routes.clear();
                    this.prepare_departure(
                        i,
                        current,
                        target,
                        &quotes,
                        parking,
                        allocator,
                        network,
                        graph,
                        &mut routes,
                    );
                }
            });
        });
        self.movement_query_shards = shards;
        self.parking_work.quotes = quotes;
    }

    // Candidate `i`'s expected questions, answered. `None` for a visit that routes nothing, as
    // most bay releases and arrivals do. O(k + Q), a pure read.
    #[allow(clippy::too_many_arguments)]
    fn prepare_visit(
        &self,
        i: usize,
        quotes: &SharedNetworkQuotes,
        parking: &ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        routes: &mut Vec<u32>,
    ) -> Option<PreparedVisit> {
        let trip = self.agents.parking_trip[i];
        if trip.retry_pending || trip.replan_needed {
            return None;
        }
        if trip.phase == ParkingTripPhase::None {
            let (current, target) = (
                self.agents.current_building[i],
                self.agents.planned_target_building[i],
            );
            return self
                .parking_departure_due(i)
                .then(|| {
                    self.prepare_departure(
                        i, current, target, quotes, parking, allocator, network, graph, routes,
                    )
                })
                .flatten();
        }
        if !trip.ready || (trip.delivery_only && trip.phase == ParkingTripPhase::PullIn) {
            return None;
        }
        let next = trip.phase.next_transition()?;
        let target = allocator
            .building_handles
            .resolve(Handle::from_bits(trip.target))?;
        let leg = next_parking_leg(trip, next, target, parking, allocator, network, graph)?;
        let quote = leg.quote(network, graph, &self.pathfind_count, Some(quotes));
        let route = quote
            .as_ref()
            .map(|quote| prepare_route(leg.key, quote, routes, network, graph));
        Some(PreparedVisit {
            leg: Some((leg.key, quote)),
            trip: None,
            route,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_departure(
        &self,
        i: usize,
        current: usize,
        target: usize,
        quotes: &SharedNetworkQuotes,
        parking: &ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        routes: &mut Vec<u32>,
    ) -> Option<PreparedVisit> {
        if current == target {
            return None;
        }
        let origin = allocator.entrances.get(current)?;
        let destination = allocator.entrances.get(target)?;
        let count = &self.pathfind_count;
        let key = LegKey::on_foot(LegEnd::Door(current), LegEnd::Door(target));
        let shared = Some(quotes);
        let walk = quote_parking_leg(
            MODE_WALK,
            origin,
            destination,
            [0.0; 2],
            network,
            graph,
            count,
            shared,
        );
        let bays = parking_trip_bays(
            self.agents.parking_trip[i].vehicle,
            self.handles.handle(i),
            self.agents.age_group[i] != AGE_CHILD,
            target,
            parking,
            allocator,
        );
        let trip = bays.map(|bays| {
            let quote = quote_parking_between_places(
                bays.0, bays.1, current, target, parking, allocator, network, graph, count, shared,
            );
            (bays, quote)
        });
        let driven = trip
            .as_ref()
            .and_then(|(_, quote)| quote.as_ref())
            .filter(|quote| !walk_is_quicker(walk.as_ref(), quote));
        let route = match (driven, walk.as_ref()) {
            (Some(quote), _) => {
                let to_car = LegKey::on_foot(LegEnd::Door(current), LegEnd::Bay(quote.source));
                Some(prepare_route(to_car, &quote.to_car, routes, network, graph))
            }
            (None, Some(walk)) => Some(prepare_route(key, walk, routes, network, graph)),
            (None, None) => None,
        };
        Some(PreparedVisit {
            leg: Some((key, walk)),
            trip,
            route,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn asked(ask: impl FnOnce(&Cell<bool>)) -> bool {
        let asked = Cell::new(false);
        ask(&asked);
        asked.get()
    }

    // No fixture reaches these: a tick's legs and sources are the same in the pass and the walk.
    #[test]
    fn an_answer_to_another_question_is_not_taken() {
        let bay = |slot| Handle::<Place>::from_parts(slot, 0);
        let on_foot = |door| LegKey::on_foot(LegEnd::Door(door), LegEnd::Bay(bay(1)));
        let visit = PreparedVisit {
            leg: Some((on_foot(1), None)),
            trip: Some(((bay(1), bay(2)), None)),
            route: Some((on_foot(1), Some((1, 2)))),
        };
        let quotes = SharedNetworkQuotes::default();
        let known = KnownVisit {
            visit: Some(&visit),
            routes: &[7, 8, 9],
            quotes: &quotes,
        };
        assert_eq!(known.route(on_foot(1)), Some(Some(&[8, 9][..])));
        assert_eq!(known.route(on_foot(2)), None);
        assert!(asked(|asked| {
            known.leg_or(on_foot(2), |_| {
                asked.set(true);
                None
            });
        }));
        for bays in [(bay(1), bay(3)), (bay(3), bay(2))] {
            assert!(asked(|asked| {
                known.trip_or(bays, |_| {
                    asked.set(true);
                    None
                });
            }));
        }
    }
}
