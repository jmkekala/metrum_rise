// SPDX-License-Identifier: GPL-2.0-only

//! Exact household-quote prefixes across matching live funding inputs and closure attempts.

use super::super::customers::{ConsumerMarket, DirectConsumerQuoteKey};
use super::*;
use crate::simulation::buildings::allocator::BuildingEntranceKey;
use std::collections::HashMap;

/// Inputs remain immutable for all closure attempts in one census. Cross-census reuse first
/// compares household accounts/access, then every seller's entrance, price and output request.
pub(super) struct Census<'a> {
    allocator: &'a BuildingAllocator,
    catalog: &'a RuntimeEconomyCatalog,
    fiscal: &'a CityFiscalPolicy,
    network: &'a TransitNetwork,
    graph: &'a RegionGraph,
}

impl<'a> Census<'a> {
    /// Binds one refresh's authoritative inputs; this contains no owned simulation state.
    pub(super) fn new(
        allocator: &'a BuildingAllocator,
        catalog: &'a RuntimeEconomyCatalog,
        fiscal: &'a CityFiscalPolicy,
        network: &'a TransitNetwork,
        graph: &'a RegionGraph,
    ) -> Self {
        Self {
            allocator,
            catalog,
            fiscal,
            network,
            graph,
        }
    }

    fn reset(&self, customers: &mut ConsumerMarket) {
        customers.reset_claims();
    }

    fn quote(&self, owner: usize, customers: &mut ConsumerMarket, units: &mut [f64]) {
        let building = &self.allocator.buildings[owner];
        // Recorded owners were checked before quotation; the census catalog cannot change.
        let profile = self
            .catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .unwrap();
        reserve_consumer_outputs(
            customers,
            units,
            owner,
            building,
            profile,
            self.fiscal,
            self.allocator,
            self.network,
            self.graph,
        );
    }
}

#[derive(Clone, Copy)]
struct Quote {
    owner: usize,
    first: usize,
    ports: usize,
    accepted: bool,
    gross_price: u32,
    entrance: Option<BuildingEntranceKey>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct OutputRequest {
    resource: u16,
    units: u32,
}

/// Retains the preceding closure attempt's exact request/outcome sequence. An unchanged
/// prefix reads cached quantities without resetting budgets, searching routes or mutating
/// receipt trees. The first changed request or decision reconstructs its accepted prefix
/// through ordinary quotation, then records the rest normally. No hashes or tolerances.
#[derive(Clone, Default)]
pub(super) struct HouseholdQuotes {
    quotes: Vec<Quote>,
    units: Vec<f64>,
    requests: Vec<OutputRequest>,
    scratch: Vec<f64>,
    cursor: usize,
    replaying: bool,
    pending: bool,
    /// Test oracle that runs ordinary budget reset and quotation for every attempt.
    #[cfg(test)]
    pub(super) disabled: bool,
    /// Requests replayed without touching the retained customer book in this census.
    #[cfg(test)]
    pub(super) reused: usize,
    /// Prefix reconstructions caused by a changed request or acceptance decision.
    #[cfg(test)]
    pub(super) rebuilt: usize,
    /// Quoted prefixes reused specifically from the preceding census's final book.
    #[cfg(test)]
    pub(super) census_reused: usize,
    // Exact request keys need comparison once across censuses, not on immutable retries.
    previous_census: bool,
    // Rejected direct-only single-output questions share a result while accounts are unchanged.
    // Keys are immutable within a census; revisions replace values without growing storage.
    rejected: HashMap<DirectConsumerQuoteKey, (u64, f64)>,
    pending_key: Option<(DirectConsumerQuoteKey, u64)>,
    pending_direct: bool,
    pending_rejected_replay: bool,
    /// Cross-owner rejected queries reused in this census.
    #[cfg(test)]
    pub(super) rejected_reused: usize,
    /// Reused rejected queries subsequently reserved normally before acceptance.
    #[cfg(test)]
    pub(super) rejected_accepted: usize,
}

impl HouseholdQuotes {
    /// Prepares at most three requests per consumer firm: imports, current local staff and
    /// local expansion. O(consumer firms + their output ports) storage; the rejected-question table clears once per census.
    pub(super) fn prepare(
        &mut self,
        firms: usize,
        ports: usize,
        maximum_ports: usize,
        retained: bool,
    ) {
        self.rejected.clear();
        self.rejected.reserve(firms);
        self.pending_key = None;
        self.pending_direct = false;
        self.pending_rejected_replay = false;
        if !retained {
            self.quotes.clear();
            self.units.clear();
            self.requests.clear();
        }
        self.quotes
            .reserve((3 * firms).saturating_sub(self.quotes.len()));
        self.units
            .reserve((3 * ports).saturating_sub(self.units.len()));
        self.requests
            .reserve((3 * ports).saturating_sub(self.requests.len()));
        self.scratch.resize(maximum_ports, -1.0);
        self.cursor = 0;
        self.replaying = retained;
        self.previous_census = retained;
        self.pending = false;
        #[cfg(test)]
        {
            self.rejected_reused = 0;
            self.rejected_accepted = 0;
            self.reused = 0;
            self.rebuilt = 0;
            self.census_reused = 0;
        }
    }

    /// Drops the recorded quote sequence after the customer book was reset, so no later
    /// preparation replays it against a book that no longer holds its claims (`SIM-02-K`: a
    /// census restarted between the consumer-market refresh and setup). O(1), keeps storage.
    pub(super) fn forget(&mut self) {
        debug_assert!(!self.pending);
        self.quotes.clear();
        self.units.clear();
        self.requests.clear();
    }

    /// Starts an exact retry in O(1), retaining the preceding accepted household book.
    pub(super) fn restart(&mut self, customers: &mut ConsumerMarket, census: &Census<'_>) {
        debug_assert!(!self.pending);
        self.cursor = 0;
        self.replaying = true;
        self.previous_census = false;
        #[cfg(test)]
        if self.disabled {
            self.quotes.clear();
            self.units.clear();
            self.requests.clear();
            self.replaying = false;
            census.reset(customers);
        }
        // Only test builds can request an uncached oracle; production retries retain it.
        let _ = (customers, census);
    }

    /// Stages one immutable owner's full-output household quote. Stable prefixes cost
    /// O(output ports); a changed request uses the ordinary exact route/account query.
    pub(super) fn reserve(
        &mut self,
        owner: usize,
        customers: &mut ConsumerMarket,
        units: &mut [f64],
        census: &Census<'_>,
    ) {
        debug_assert!(!self.pending);
        self.pending_key = None;
        self.pending_direct = false;
        self.pending_rejected_replay = false;
        let building = &census.allocator.buildings[owner];
        let profile = census
            .catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .unwrap();
        let area = output_area_scale_for_building(building, profile);
        let gross_price =
            (profile.unit_price_currency * (1.0 + census.fiscal.household_vat_rate)).to_bits();
        let entrance = census
            .allocator
            .entrances
            .get(owner)
            .map(BuildingEntranceKey::new);
        let requests = || {
            profile.outputs.iter().map(|output| OutputRequest {
                resource: output.resource_runtime_id,
                units: (profile.net_output_units_per_day(output) * area).to_bits(),
            })
        };
        if self.replaying
            && !self.quotes.get(self.cursor).is_some_and(|quote| {
                quote.owner == owner
                    && (!self.previous_census
                        || (quote.ports == units.len()
                            && quote.gross_price == gross_price
                            && quote.entrance == entrance
                            && requests()
                                .eq(self.requests[quote.first..quote.first + quote.ports]
                                    .iter()
                                    .copied())))
            })
        {
            self.rebuild_prefix(customers, census);
            self.truncate_prefix();
        }
        if self.replaying {
            let quote = self.quotes[self.cursor];
            units.copy_from_slice(&self.units[quote.first..quote.first + quote.ports]);
            #[cfg(test)]
            {
                self.reused += 1;
                self.census_reused += usize::from(self.previous_census);
            }
        } else {
            #[cfg(test)]
            let reuse_enabled = !self.disabled;
            #[cfg(not(test))]
            let reuse_enabled = true;
            if reuse_enabled && profile.outputs.len() == 1 {
                let output = &profile.outputs[0];
                self.pending_key = census.allocator.entrances.get(owner).and_then(|entrance| {
                    customers
                        .direct_quote_key(
                            entrance,
                            output.resource_runtime_id,
                            f64::from(profile.net_output_units_per_day(output) * area),
                            f32::from_bits(gross_price),
                            census.network,
                            census.graph,
                        )
                        .map(|key| (key, customers.quote_revision()))
                });
            }
            if let Some((key, revision)) = self.pending_key
                && let Some(&(saved_revision, value)) = self.rejected.get(&key)
                && revision == saved_revision
            {
                units[0] = value;
                self.pending_rejected_replay = true;
                #[cfg(test)]
                {
                    self.rejected_reused += 1;
                }
            } else {
                census.quote(owner, customers, units);
                self.pending_direct =
                    self.pending_key.is_some() && customers.last_claim_was_direct();
            }
            assert!(
                self.quotes.len() < self.quotes.capacity()
                    && self.units.len() + units.len() <= self.units.capacity()
                    && self.requests.len() + units.len() <= self.requests.capacity(),
                "household quote replay requires census preparation"
            );
            self.quotes.push(Quote {
                owner,
                first: self.units.len(),
                ports: units.len(),
                accepted: false,
                gross_price,
                entrance,
            });
            self.units.extend_from_slice(units);
            self.requests.extend(requests());
        }
        self.pending = true;
    }

    /// Confirms the financing decision. A changed outcome first restores the exact preceding
    /// budget state and restages this request, so acceptance/rejection keeps normal rollback.
    pub(super) fn finish(
        &mut self,
        accepted: bool,
        customers: &mut ConsumerMarket,
        census: &Census<'_>,
    ) {
        debug_assert!(self.pending);
        let quote = self.quotes[self.cursor];
        if self.replaying && quote.accepted != accepted {
            self.rebuild_prefix(customers, census);
            census.quote(quote.owner, customers, &mut self.scratch[..quote.ports]);
            debug_assert_eq!(
                &self.scratch[..quote.ports],
                &self.units[quote.first..quote.first + quote.ports]
            );
            self.quotes.truncate(self.cursor + 1);
            self.units.truncate(quote.first + quote.ports);
            self.requests.truncate(quote.first + quote.ports);
        }
        if !self.replaying {
            if accepted {
                if self.pending_rejected_replay {
                    // A scalar proof has no receipts. Materialize them in the original
                    // family order before publishing this owner's accepted purchases.
                    census.quote(quote.owner, customers, &mut self.scratch[..quote.ports]);
                    debug_assert_eq!(
                        &self.scratch[..quote.ports],
                        &self.units[quote.first..quote.first + quote.ports]
                    );
                    #[cfg(test)]
                    {
                        self.rejected_accepted += 1;
                    }
                }
                customers.accept_growing_for(quote.owner);
            } else if !self.pending_rejected_replay {
                customers.reject();
                if self.pending_direct
                    && let Some((key, revision)) = self.pending_key
                    && revision == customers.quote_revision()
                {
                    assert!(
                        self.rejected.len() < self.rejected.capacity()
                            || self.rejected.contains_key(&key),
                        "rejected household quotes require census preparation"
                    );
                    self.rejected
                        .insert(key, (revision, self.units[quote.first]));
                }
            }
            self.quotes[self.cursor].accepted = accepted;
        }
        self.cursor += 1;
        self.pending = false;
    }

    /// Removes any skipped trailing requests before the customer book can be published.
    pub(super) fn finish_pass(&mut self, customers: &mut ConsumerMarket, census: &Census<'_>) {
        debug_assert!(!self.pending);
        if self.replaying && self.cursor != self.quotes.len() {
            self.rebuild_prefix(customers, census);
            self.truncate_prefix();
        }
        debug_assert_eq!(self.cursor, self.quotes.len());
    }

    fn rebuild_prefix(&mut self, customers: &mut ConsumerMarket, census: &Census<'_>) {
        census.reset(customers);
        for quote in &self.quotes[..self.cursor] {
            if !quote.accepted {
                continue;
            }
            census.quote(quote.owner, customers, &mut self.scratch[..quote.ports]);
            debug_assert_eq!(
                &self.scratch[..quote.ports],
                &self.units[quote.first..quote.first + quote.ports]
            );
            customers.accept_growing_for(quote.owner);
        }
        self.replaying = false;
        #[cfg(test)]
        {
            self.rebuilt += 1;
        }
    }

    fn truncate_prefix(&mut self) {
        let end = self
            .quotes
            .get(self.cursor)
            .map_or(self.units.len(), |quote| quote.first);
        self.quotes.truncate(self.cursor);
        self.units.truncate(end);
        self.requests.truncate(end);
    }
}
