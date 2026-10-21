// SPDX-License-Identifier: GPL-2.0-only

//! Shared input-purchase quantities and the financing effects of changing their suppliers.

use crate::simulation::economy::operation::InputRequirement;

mod buyers;
mod repricing;
mod revalidation;
mod routing;
pub(crate) use routing::{BusinessMarket, PreparedBusinessSite, SeedingLayout};

#[derive(Clone, Copy, Debug)]
struct Account {
    buyer: usize,
    resource: u16,
    units: f64,
    goods: f64,
    carrier: f64,
    stock_exposure: f64,
    batch: f32,
    goods_multiplier: f32,
    next_buyer: usize,
    first_receipt: usize,
}

#[derive(Clone, Copy, Debug)]
struct Receipt {
    account: usize,
    units: f64,
    margin: f64,
    capital: f64,
    goods: f64,
    quote: [f32; 2],
    input_origin: bool,
    next: usize,
    seller: usize,
    previous: usize,
    next_account: usize,
    previous_account: usize,
}

#[derive(Clone, Copy, Debug)]
struct Change {
    account: usize,
    previous_units: f64,
    acquired: Receipt,
}

/// Sparse business input accounts with one shared profit/capital balance per buyer.
/// A seed describes an evaluated operation; callers must validate the complete mutually
/// dependent set before publishing it as established demand.
#[derive(Clone, Debug, Default)]
pub(crate) struct BusinessBudgets {
    accounts: Vec<Account>,
    buyer_accounts: Vec<usize>,
    buyer_tails: Vec<usize>,
    finance: Vec<[f64; 2]>,
    staged: Vec<Change>,
    staged_slots: Vec<usize>,
    staged_finance: Vec<(usize, [f64; 2])>,
    finance_staged: Vec<bool>,
    committed: Vec<Receipt>,
    owners: Vec<usize>,
    free_receipt: usize,
    releasing_owner: Option<usize>,
    // Only the bounded reseeding callback may overwrite this buyer's withdrawn tranches.
    replacing_inputs: Option<(usize, usize, usize)>,
    repricing: repricing::Repricing,
    revalidation: revalidation::Revalidation,
}

impl BusinessBudgets {
    /// Copies accepted accounts, retaining warm journal storage; test forks only, as planning
    /// lends the live book.
    #[cfg(test)]
    fn copy_reservations_from(&mut self, source: &Self) {
        debug_assert!(source.staged.is_empty());
        debug_assert!(!self.revalidation.is_active() && !source.revalidation.is_active());
        self.accounts.clone_from(&source.accounts);
        self.buyer_accounts.clone_from(&source.buyer_accounts);
        self.buyer_tails.clone_from(&source.buyer_tails);
        self.finance.clone_from(&source.finance);
        self.committed.clone_from(&source.committed);
        self.committed.reserve(
            source
                .committed
                .capacity()
                .saturating_sub(self.committed.len()),
        );
        self.owners.clone_from(&source.owners);
        self.free_receipt = source.free_receipt;
        self.releasing_owner = None;
        self.staged.clear();
        self.staged.reserve(self.accounts.len());
        self.staged_slots.clone_from(&source.staged_slots);
        self.staged_finance.clear();
        self.staged_finance.reserve(self.finance.len());
        self.finance_staged.clone_from(&source.finance_staged);
        crate::debug::work::add(
            crate::debug::work::Work::ReservationRowsCopied,
            (self.accounts.len()
                + 2 * self.buyer_accounts.len()
                + 2 * self.finance.len()
                + self.committed.len()
                + self.owners.len()
                + self.staged_slots.len()) as u64,
        );
    }

    /// Gives this copy of `live` the capacity of the receipt arena a census refills
    /// (`SIM-02-K`, [`crate::utils::storage::reserve_capacity`]). O(1) without growth.
    pub(crate) fn reserve_like(&mut self, live: &Self) {
        crate::utils::storage::reserve_capacity(&mut self.committed, live.committed.capacity());
    }

    /// Prepares sparse storage before visiting input ports. `input_ports` includes all live
    /// and projected account slots. Output ports size the initial receipt estimate; financial
    /// fragmentation can require a larger arena, prepared between complete matching attempts.
    pub(crate) fn reset(
        &mut self,
        buyers: usize,
        input_ports: usize,
        sellers: usize,
        output_ports: usize,
    ) {
        debug_assert!(!self.revalidation.is_active());
        self.accounts.clear();
        self.accounts.reserve(input_ports);
        self.buyer_accounts.resize(buyers, usize::MAX);
        self.buyer_accounts.fill(usize::MAX);
        self.buyer_tails.resize(buyers, usize::MAX);
        self.buyer_tails.fill(usize::MAX);
        self.finance.resize(buyers, [0.0; 2]);
        self.finance.fill([0.0; 2]);
        self.staged.clear();
        // Each account and buyer is journaled once, even across repeated output claims.
        self.staged.reserve(input_ports);
        self.staged_slots.clear();
        self.staged_slots.reserve(input_ports);
        self.staged_finance.clear();
        self.staged_finance.reserve(buyers);
        self.finance_staged.resize(buyers, false);
        self.finance_staged.fill(false);
        self.committed.clear();
        // The census's receipt estimate keeps spare capacity, as the arenas grown by request do
        // (`SIM-02-K`): reserved exactly, a census whose estimate passed the last one's by a few
        // receipts doubled the arena, and the next planning slot to lend the book grew every
        // journal it sizes.
        let receipts = crate::utils::storage::spare_capacity(
            2 * (input_ports + output_ports),
            self.committed.capacity(),
        );
        crate::utils::storage::reserve_cleared(&mut self.committed, receipts);
        self.owners.resize(sellers, usize::MAX);
        self.owners.fill(usize::MAX);
        self.free_receipt = usize::MAX;
        self.releasing_owner = None;
    }

    /// Stores the evaluated operation's recurring-profit and working-capital surplus.
    /// Fiscal invoices can use infinite limits; private budgets must come from sales and assets.
    pub(crate) fn set_finance(&mut self, buyer: usize, headroom: [f64; 2]) {
        self.journal_finance(buyer);
        self.finance[buyer] = headroom;
    }

    /// Adds one source tranche from an evaluated input quote. The caller caps `units` by
    /// established staffed consumption; existing local purchases and the outside residual
    /// retain separate prices. `input` describes the complete supported operation, so stock
    /// exposure includes both its buffer and retained inventory under supplier substitution.
    /// Returns a financial tranche slot; routing maps it to the buyer's prepared input port.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_input(
        &mut self,
        buyer: usize,
        input: InputRequirement,
        goods_price: f32,
        carrier_quote: f32,
        inventory: f32,
        units: f64,
        goods_multiplier: f32,
    ) -> Option<usize> {
        if input.daily_units <= 0.0
            || !input.daily_units.is_finite()
            || !goods_price.is_finite()
            || goods_price < 0.0
            || !carrier_quote.is_finite()
            || carrier_quote < 0.0
            || !goods_multiplier.is_finite()
            || goods_multiplier < 0.0
            || !input.buffer_units.is_finite()
            || !inventory.is_finite()
            || units <= 0.0
        {
            return None;
        }
        let batch = input.replenishment_batch_units?;
        if batch <= 0.0 || !batch.is_finite() {
            return None;
        }
        let daily = f64::from(input.daily_units);
        let account = Account {
            buyer,
            resource: input.resource,
            units: units.min(daily),
            goods: f64::from(goods_price),
            carrier: f64::from(carrier_quote) / f64::from(batch),
            stock_exposure: (f64::from(input.buffer_units) - f64::from(inventory)) / daily,
            batch,
            goods_multiplier,
            next_buyer: usize::MAX,
            first_receipt: usize::MAX,
        };
        if let Some((owner, next, remaining)) = &mut self.replacing_inputs {
            assert_eq!(*owner, buyer);
            assert!(*remaining > 0, "input seed must fit its preflight count");
            *remaining -= 1;
            let slot = *next;
            if slot != usize::MAX {
                *next = self.accounts[slot].next_buyer;
                let next_buyer = *next;
                debug_assert!(
                    self.accounts[slot].units == 0.0
                        && self.accounts[slot].first_receipt == usize::MAX
                );
                self.journal_account(slot);
                self.accounts[slot] = Account {
                    next_buyer,
                    ..account
                };
                return Some(slot);
            }
        }
        let slot = self.accounts.len();
        assert!(
            !self.revalidation.is_active() || slot < self.accounts.capacity(),
            "prepare replacement account storage before beginning its proof"
        );
        self.journal_buyer(buyer);
        self.accounts.push(account);
        let previous = self.buyer_tails[buyer];
        if previous == usize::MAX {
            self.buyer_accounts[buyer] = slot;
        } else {
            self.journal_account(previous);
            self.accounts[previous].next_buyer = slot;
        }
        self.buyer_tails[buyer] = slot;
        self.staged_slots.push(usize::MAX);
        Some(slot)
    }

    /// Prepares appended input tranches and their journals between complete operation proofs.
    pub(super) fn reserve_accounts(&mut self, required: usize) {
        debug_assert!(!self.revalidation.is_active() && self.staged.is_empty());
        self.accounts
            .reserve(required.saturating_sub(self.accounts.len()));
        self.prepare_revalidation();
        self.prepare_repricing();
    }

    /// Exact preflight for new input accounts; capacity cannot decide economic feasibility.
    /// On exhaustion the caller must reject the outer proof, prepare storage, and replay it.
    pub(super) fn check_replacement_capacity(
        &self,
        buyer: usize,
        tranches: usize,
    ) -> Result<(), usize> {
        let mut existing = 0;
        let mut slot = self.buyer_accounts[buyer];
        while slot != usize::MAX {
            existing += 1;
            slot = self.accounts[slot].next_buyer;
        }
        let required = self.accounts.len() + tranches.saturating_sub(existing);
        if required > self.accounts.capacity() {
            Err(required)
        } else {
            Ok(())
        }
    }

    /// Reuses this buyer's withdrawn account slots in quote order before appending more.
    /// Repeated accepted revalidation therefore retains its peak tranche count, not history.
    pub(super) fn replace_inputs(
        &mut self,
        buyer: usize,
        tranches: usize,
        seed: impl FnOnce(&mut Self),
    ) {
        debug_assert!(
            self.revalidation.is_active()
                && self.staged.is_empty()
                && self.replacing_inputs.is_none()
        );
        self.replacing_inputs = Some((buyer, self.buyer_accounts[buyer], tranches));
        seed(self);
        self.replacing_inputs = None;
        debug_assert!(self.staged.is_empty());
    }

    fn stage(&mut self, slot: usize) -> usize {
        let account = self.accounts[slot];
        self.journal_account(slot);
        self.journal_finance(account.buyer);
        if !self.finance_staged[account.buyer] {
            self.staged_finance
                .push((account.buyer, self.finance[account.buyer]));
            self.finance_staged[account.buyer] = true;
        }
        if self.staged_slots[slot] == usize::MAX {
            self.staged_slots[slot] = self.staged.len();
            self.staged.push(Change {
                account: slot,
                previous_units: account.units,
                acquired: Receipt {
                    account: slot,
                    units: 0.0,
                    margin: 0.0,
                    capital: 0.0,
                    goods: 0.0,
                    quote: [0.0; 2],
                    input_origin: false,
                    next: usize::MAX,
                    seller: usize::MAX,
                    previous: usize::MAX,
                    next_account: usize::MAX,
                    previous_account: usize::MAX,
                },
            });
        }
        self.staged_slots[slot]
    }

    /// Stages a seller against both shared financing constraints, preserving positive
    /// recurring profit and nonnegative working-capital surplus. Cheaper goods/freight never
    /// credit savings before that delivery exists; actual prices enter through repricing.
    /// More expensive goods/freight and lost stock value debit proven financing, affine in
    /// claimed units at the operation's already-quantized batch.
    pub(crate) fn claim_opportunity(
        &mut self,
        slot: usize,
        requested: f64,
        seller_price: f32,
        carrier_quote: f32,
    ) -> f64 {
        self.claim_priced(slot, requested, seller_price, carrier_quote, true)
    }

    /// Reserves a coupled replacement's finite quantity before its complete-set proof.
    /// Temporary negative financing cannot escape the enclosing revalidation transaction.
    pub(super) fn claim_joint_opportunity(
        &mut self,
        slot: usize,
        requested: f64,
        seller_price: f32,
        carrier_quote: f32,
    ) -> f64 {
        debug_assert!(self.revalidation.is_active());
        self.claim_priced(slot, requested, seller_price, carrier_quote, false)
    }

    /// Whether a seller can claim any units at a proven lower bound on its carrier price.
    /// Claims only shrink as the carrier quote grows, so `false` excludes every actual route.
    /// Pure; O(1), including the same physical-quantity rounding as an ordinary claim.
    pub(crate) fn claimable_at_carrier_lower_bound(
        &self,
        slot: usize,
        requested: f64,
        seller_price: f32,
        minimum_carrier: f32,
        require_finance: bool,
    ) -> bool {
        self.priced_claim(
            slot,
            requested,
            seller_price,
            minimum_carrier,
            require_finance,
        )
        .is_some()
    }

    /// Conservative existence test for any request at this exact goods price and free freight.
    /// Omits physical f32 rounding and strict-budget corrections: a smaller unrounded f64
    /// request may succeed when a larger financing-limited request rounds to zero.
    pub(super) fn potentially_claimable(&self, slot: usize, seller_price: f32) -> bool {
        let account = self.accounts[slot];
        let finance = self.finance[account.buyer];
        if !(finance[0] > 0.0 && finance[1] >= 0.0) {
            return false;
        }
        let Some((mut units, margin, capital, _)) =
            self.priced_claim(slot, account.units, seller_price, 0.0, false)
        else {
            return false;
        };
        if margin > 0.0 && finance[0].is_finite() {
            units = units.min(finance[0] / margin);
        }
        if capital > 0.0 && finance[1].is_finite() {
            units = units.min(finance[1] / capital);
        }
        units > 0.0 && account.units - units != account.units
    }

    // Units, margin delta, capital delta and unit goods price of a claim, without staging it.
    // Both deltas are non-decreasing in the carrier quote, so the units are non-increasing.
    fn priced_claim(
        &self,
        slot: usize,
        requested: f64,
        seller_price: f32,
        carrier_quote: f32,
        require_finance: bool,
    ) -> Option<(f64, f64, f64, f32)> {
        let account = self.accounts[slot];
        let goods = seller_price * account.goods_multiplier;
        if requested <= 0.0
            || !goods.is_finite()
            || goods < 0.0
            || !carrier_quote.is_finite()
            || carrier_quote < 0.0
        {
            return None;
        }
        let finance = self.finance[account.buyer];
        // Price deltas are nonnegative, so no offer can repair an already infeasible buyer.
        if require_finance && !(finance[0] > 0.0 && finance[1] >= 0.0) {
            return None;
        }
        let goods_delta = f64::from(goods) - account.goods;
        let carrier_delta = f64::from(carrier_quote) / f64::from(account.batch) - account.carrier;
        let margin_delta = (goods_delta + carrier_delta).max(0.0);
        let capital_delta = (goods_delta * account.stock_exposure + carrier_delta).max(0.0);
        let quantity_limit = requested.min(account.units);
        let mut units = quantity_limit;
        if require_finance {
            if margin_delta > 0.0 && finance[0].is_finite() {
                units = units.min((finance[0] / margin_delta).next_down());
            }
            if capital_delta > 0.0 && finance[1].is_finite() {
                units = units.min(finance[1] / capital_delta);
            }
            // A financing-limited delivery must be representable by physical recipe/storage
            // quantities. Round down before committing money, not later when material is moved.
            if units < quantity_limit {
                let physical = units as f32;
                units = f64::from(if f64::from(physical) > units {
                    physical.next_down()
                } else {
                    physical
                });
            }
            // Division and multiplication may round in opposite directions. Step to the next
            // representable quantity until the original strict/non-strict contracts both hold.
            // This is a floating-point boundary correction, not an economic tolerance or cutoff.
            while units > 0.0
                && (finance[0] - units * margin_delta <= 0.0
                    || finance[1] - units * capital_delta < 0.0)
            {
                units = f64::from((units as f32).next_down());
            }
        }
        if units <= 0.0 || account.units - units == account.units {
            return None;
        }
        Some((units, margin_delta, capital_delta, goods))
    }

    fn claim_priced(
        &mut self,
        slot: usize,
        requested: f64,
        seller_price: f32,
        carrier_quote: f32,
        require_finance: bool,
    ) -> f64 {
        let Some((units, margin_delta, capital_delta, goods)) = self.priced_claim(
            slot,
            requested,
            seller_price,
            carrier_quote,
            require_finance,
        ) else {
            return 0.0;
        };
        let account = self.accounts[slot];
        let finance = self.finance[account.buyer];
        let acquired = Receipt {
            account: slot,
            units,
            margin: units * margin_delta,
            capital: units * capital_delta,
            goods: units * f64::from(goods),
            quote: [goods, carrier_quote],
            input_origin: false,
            next: usize::MAX,
            seller: usize::MAX,
            previous: usize::MAX,
            next_account: usize::MAX,
            previous_account: usize::MAX,
        };
        let staged = self.stage(slot);
        let receipt = &mut self.staged[staged].acquired;
        debug_assert!(receipt.units == 0.0 || receipt.quote == acquired.quote);
        receipt.quote = acquired.quote;
        receipt.units += acquired.units;
        receipt.margin += acquired.margin;
        receipt.capital += acquired.capital;
        receipt.goods += acquired.goods;
        self.accounts[slot].units -= units;
        self.finance[account.buyer] = [finance[0] - acquired.margin, finance[1] - acquired.capital];
        units
    }

    /// Seeds an already-quoted local flow without spending its buyer's headroom twice.
    /// The whole provisional financing set must still be validated before publication.
    pub(crate) fn seed_for(&mut self, source: usize, slot: usize) {
        self.journal_account(slot);
        self.journal_owner(source);
        let account = self.accounts[slot];
        self.insert_receipt(
            source,
            usize::MAX,
            Receipt {
                account: slot,
                units: account.units,
                margin: 0.0,
                capital: 0.0,
                goods: account.units * account.goods,
                quote: [
                    account.goods as f32,
                    (account.carrier * f64::from(account.batch)) as f32,
                ],
                input_origin: true,
                next: usize::MAX,
                seller: usize::MAX,
                previous: usize::MAX,
                next_account: usize::MAX,
                previous_account: usize::MAX,
            },
        );
        self.accounts[slot].units = 0.0;
    }

    fn append_receipt(&mut self, receipt: Receipt) -> usize {
        let index = self.committed.len();
        self.committed.push(receipt);
        self.revalidation.follow_receipts(self.committed.capacity());
        index
    }

    /// Existing seller claims for one resource, capped by its executable output quantity.
    pub(crate) fn sales(&self, owner: usize, resource: u16, limit: f64) -> (f64, f64) {
        let mut remaining = limit;
        let mut sold = 0.0;
        let mut revenue = 0.0;
        // Existing input commitments precede additional outside-customer opportunities.
        for input_origin in [true, false] {
            let mut index = self.owners[owner];
            while remaining > 0.0 && index != usize::MAX {
                let receipt = self.committed[index];
                index = receipt.next;
                if receipt.input_origin != input_origin
                    || self.accounts[receipt.account].resource != resource
                {
                    continue;
                }
                let units = remaining.min(receipt.units);
                remaining -= units;
                sold += units;
                revenue += receipt.goods * (units / receipt.units);
            }
        }
        (sold, revenue)
    }

    /// Grows retained storage outside seller loops after an exact capacity request. The next
    /// proof replays from its original accounts; capacity never decides economic feasibility.
    pub(crate) fn reserve_receipts(&mut self, required: usize) {
        debug_assert!(!self.revalidation.is_active());
        self.committed
            .reserve(required.saturating_sub(self.committed.len()));
        self.revalidation.follow_receipts(self.committed.capacity());
        self.prepare_repricing();
    }

    /// Adds newly financed customers without growing the receipt arena inside a seller loop.
    /// On exhaustion, leaves the staged quote untouched and returns the exact required size.
    /// The enclosing proof must prepare more storage and replay before publishing any result.
    pub(crate) fn try_extend_for(&mut self, owner: usize) -> Result<(), usize> {
        debug_assert!(self.releasing_owner.is_none());
        let added = self
            .staged
            .iter()
            .filter(|change| change.acquired.units > 0.0)
            .count();
        let required = self.committed.len() + added;
        if required > self.committed.capacity() {
            return Err(required);
        }
        self.journal_owner(owner);
        for staged in 0..self.staged.len() {
            let receipt = self.staged[staged].acquired;
            if receipt.units <= 0.0 {
                continue;
            }
            self.insert_receipt(owner, usize::MAX, receipt);
        }
        self.finish();
        Ok(())
    }

    /// Releases only this seller's purchases; rejection restores exact pre-release numbers.
    pub(crate) fn release_owner(&mut self, owner: usize) {
        self.reject();
        self.releasing_owner = Some(owner);
        let mut index = self.owners.get(owner).copied().unwrap_or(usize::MAX);
        while index != usize::MAX {
            let receipt = self.committed[index];
            let account = self.accounts[receipt.account];
            let finance = self.finance[account.buyer];
            self.stage(receipt.account);
            self.accounts[receipt.account].units += receipt.units;
            self.finance[account.buyer] =
                [finance[0] + receipt.margin, finance[1] + receipt.capital];
            index = receipt.next;
        }
    }

    /// Retains seller receipts only if every affected buyer remains financed. A rejected
    /// commit leaves the quote staged so the caller can restore it with `reject`.
    /// Test harness: runtime proofs commit through `try_extend_for`/`commit_joint_for`.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn accept_for(&mut self, owner: usize) -> bool {
        if !self.can_accept() {
            return false;
        }
        self.commit_for(owner);
        true
    }

    /// Commits an inner quote provisionally; the enclosing joint proof must validate all
    /// touched buyers before accepting its outer journal.
    pub(super) fn commit_joint_for(&mut self, owner: usize) -> Result<(), usize> {
        debug_assert!(self.revalidation.is_active());
        let added = self
            .staged
            .iter()
            .filter(|change| change.acquired.units > 0.0)
            .count();
        let mut reusable = 0;
        for first in [self.owners[owner], self.free_receipt] {
            let mut index = first;
            while index != usize::MAX && reusable < added {
                reusable += 1;
                index = self.committed[index].next;
            }
        }
        let required = self.committed.len() + added.saturating_sub(reusable);
        if required > self.committed.capacity() {
            return Err(required);
        }
        self.commit_for(owner);
        Ok(())
    }

    fn commit_for(&mut self, owner: usize) {
        debug_assert!(
            self.releasing_owner
                .is_none_or(|released| released == owner)
        );
        debug_assert!(self.owners[owner] == usize::MAX || self.releasing_owner == Some(owner));
        self.retire(owner);
        self.releasing_owner = None;
        let mut last = usize::MAX;
        for staged in 0..self.staged.len() {
            let receipt = self.staged[staged].acquired;
            if receipt.units <= 0.0 {
                continue;
            }
            last = self.insert_receipt(owner, last, receipt);
        }
        self.finish();
    }

    fn can_accept(&self) -> bool {
        self.staged_finance
            .iter()
            .all(|&(buyer, _)| self.finance[buyer][0] > 0.0 && self.finance[buyer][1] >= 0.0)
    }

    fn retire(&mut self, owner: usize) {
        self.journal_owner(owner);
        let mut index = self.owners[owner];
        while index != usize::MAX {
            let next = self.committed[index].next;
            self.remove_receipt(index);
            index = next;
        }
        self.owners[owner] = usize::MAX;
    }

    fn finish(&mut self) {
        if let Some(owner) = self.releasing_owner.take() {
            self.retire(owner);
        }
        for change in &self.staged {
            self.staged_slots[change.account] = usize::MAX;
        }
        self.staged.clear();
        for &(buyer, _) in &self.staged_finance {
            self.finance_staged[buyer] = false;
        }
        self.staged_finance.clear();
    }

    /// Restores each account and shared balance from its first-touch snapshot, without drift.
    pub(crate) fn reject(&mut self) {
        while let Some(change) = self.staged.pop() {
            self.accounts[change.account].units = change.previous_units;
            self.staged_slots[change.account] = usize::MAX;
        }
        for (buyer, previous) in self.staged_finance.drain(..) {
            self.finance[buyer] = previous;
            self.finance_staged[buyer] = false;
        }
        self.releasing_owner = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(resource: u16, inventory: f32, book: &mut BusinessBudgets) -> usize {
        book.push_input(
            0,
            InputRequirement {
                resource,
                daily_units: 100.0,
                buffer_units: 400.0,
                replenishment_batch_units: Some(50.0),
            },
            10.0,
            50.0,
            inventory,
            100.0,
            1.0,
        )
        .unwrap()
    }

    #[test]
    fn carrier_lower_bound_never_excludes_a_fundable_actual_quote() {
        let mut book = BusinessBudgets::default();
        book.reset(1, 1, 1, 1);
        let slot = input(1, 1_000.0, &mut book);
        for finance in [
            [0.0, 0.0],
            [f64::MIN_POSITIVE, 0.0],
            [0.01, 0.01],
            [100.0, 120.0],
            [f64::INFINITY, f64::INFINITY],
        ] {
            book.set_finance(0, finance);
            for requested in [f64::MIN_POSITIVE, 0.001, 1.0, 100.0] {
                for goods in [0.0, 9.0, 10.0, 10.000_001, 12.0] {
                    for lower in [0.0_f32, 4.0, 49.999_996, 50.0, 50.000_004, 100.0] {
                        for actual in [lower, lower.next_up(), lower + 1.0, lower + 10_000.0] {
                            for require_finance in [false, true] {
                                if book
                                    .priced_claim(slot, requested, goods, actual, require_finance)
                                    .is_some()
                                {
                                    assert!(book.claimable_at_carrier_lower_bound(
                                        slot,
                                        requested,
                                        goods,
                                        lower,
                                        require_finance
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(book.staged.is_empty());
    }

    #[test]
    fn price_eligibility_never_excludes_smaller_or_physical_boundary_requests() {
        let mut book = BusinessBudgets::default();
        book.reset(1, 1, 1, 1);
        let slot = input(1, 1_000.0, &mut book);
        for remaining in [1e-100, 1e-45, 0.001, 100.0, 1e30] {
            book.accounts[slot].units = remaining;
            for exposure in [-10.0, 0.0, 0.1, 4.0] {
                book.accounts[slot].stock_exposure = exposure;
                for headroom in [
                    [0.0, 0.0],
                    [1e-110, 1e-110],
                    [1e-100, 1e-100],
                    [f64::MIN_POSITIVE, 0.0],
                    [0.01, 0.01],
                    [100.0, 120.0],
                    [f64::INFINITY, f64::INFINITY],
                ] {
                    book.set_finance(0, headroom);
                    for price in [0.0, 9.0, 10.0, 10.000_001, 12.0] {
                        for request in [
                            f64::MIN_POSITIVE,
                            remaining * 0.001,
                            remaining * 0.5,
                            remaining,
                        ] {
                            for carrier in [0.0, 0.01, 50.0, 100.0] {
                                if book
                                    .priced_claim(slot, request, price, carrier, true)
                                    .is_some()
                                {
                                    assert!(
                                        book.potentially_claimable(slot, price),
                                        "{remaining} {exposure} {headroom:?} {price} {request} {carrier}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        book.accounts[slot].units = 1e-100;
        book.accounts[slot].goods = 0.0;
        book.accounts[slot].carrier = 0.0;
        book.accounts[slot].stock_exposure = 1.0;
        book.set_finance(0, [5e-101, 5e-101]);
        assert!(!book.claimable_at_carrier_lower_bound(slot, 1e-100, 1.0, 0.0, true));
        assert!(book.claimable_at_carrier_lower_bound(slot, 1e-101, 1.0, 0.0, true));
        assert!(book.potentially_claimable(slot, 1.0));
    }

    #[test]
    fn repeated_accepted_replacements_reuse_retired_receipts() {
        let mut book = BusinessBudgets::default();
        book.reset(1, 2, 1, 2);
        book.set_finance(0, [1_000.0, 10_000.0]);
        let a = input(1, 0.0, &mut book);
        let b = input(2, 0.0, &mut book);
        for slot in [a, b] {
            assert_eq!(book.claim_opportunity(slot, 100.0, 10.0, 50.0), 100.0);
        }
        assert!(book.accept_for(0));
        let capacity = book.committed.capacity();
        for _ in 0..256 {
            book.release_owner(0);
            for slot in [a, b] {
                assert_eq!(book.claim_opportunity(slot, 100.0, 10.0, 50.0), 100.0);
            }
            assert!(book.accept_for(0));
            assert_eq!(book.committed.len(), 2);
            assert_eq!(book.committed.capacity(), capacity);
            assert!(book.accounts.iter().all(|account| account.units == 0.0));
        }
        book.release_owner(0);
        assert!(book.accounts.iter().all(|account| account.units == 100.0));
        book.reject();
        assert!(book.accounts.iter().all(|account| account.units == 0.0));
    }

    #[test]
    fn interleaved_recipe_claims_keep_bounded_first_touch_journals() {
        let mut book = BusinessBudgets::default();
        book.reset(1, 2, 1, 2);
        book.set_finance(0, [1_000.0, 10_000.0]);
        let a = input(1, 0.0, &mut book);
        let b = input(2, 0.0, &mut book);
        let capacities = (book.staged.capacity(), book.staged_finance.capacity());
        for _ in 0..100 {
            assert_eq!(book.claim_opportunity(a, 1.0, 11.0, 51.0), 1.0);
            assert_eq!(book.claim_opportunity(b, 1.0, 9.0, 49.0), 1.0);
        }
        assert_eq!((book.staged.len(), book.staged_finance.len()), (2, 1));
        assert_eq!(
            (book.staged.capacity(), book.staged_finance.capacity()),
            capacities
        );
        book.reject();
        assert_eq!(book.finance[0], [1_000.0, 10_000.0]);
        assert!(book.accounts.iter().all(|account| account.units == 100.0));
    }

    #[test]
    fn recipe_inputs_share_profit_and_capital_without_credit_from_owner_equity() {
        let mut book = BusinessBudgets::default();
        book.reset(1, 2, 2, 2);
        book.set_finance(0, [100.0, 1_000.0]);
        let a = input(1, 0.0, &mut book);
        let b = input(2, 0.0, &mut book);
        assert_eq!(book.claim_opportunity(a, 40.0, 12.0, 50.0), 40.0);
        assert!(book.accept_for(0));
        let remainder = book.claim_opportunity(b, 100.0, 12.0, 50.0);
        assert!(remainder < 10.0 && remainder > 9.99);
        assert!(book.finance[0][0] > 0.0);
        book.reject();
        assert_eq!(book.finance[0], [20.0, 680.0]);
        book.set_finance(0, [0.0, 1_000_000.0]);
        assert_eq!(
            book.claim_opportunity(b, 100.0, 12.0, 50.0),
            0.0,
            "capital alone never creates recurring purchasing power"
        );
    }

    #[test]
    fn cheaper_stock_can_limit_capital_and_rejected_replacements_restore_both_accounts() {
        let mut book = BusinessBudgets::default();
        book.reset(1, 2, 2, 2);
        book.set_finance(0, [100.0, 120.0]);
        let a = input(1, 1_000.0, &mut book);
        let b = input(2, 0.0, &mut book);
        // One cheaper unit loses ten currency of stock assets and saves four of buffer;
        // its goods saving is not credited to recurring margin before delivery.
        assert_eq!(book.claim_opportunity(a, 100.0, 9.0, 50.0), 20.0);
        assert!(book.accept_for(0));
        assert_eq!(book.finance[0], [100.0, 0.0]);
        assert_eq!(book.claim_opportunity(b, 10.0, 11.0, 50.0), 0.0);
        let initial_units: Vec<_> = book.accounts.iter().map(|a| a.units.to_bits()).collect();
        let initial_finance = book.finance[0].map(f64::to_bits);
        for _ in 0..32 {
            book.release_owner(0);
            assert_eq!(book.claim_opportunity(a, 10.0, 9.5, 50.0), 10.0);
            assert_eq!(book.claim_opportunity(b, 10.0, 11.0, 50.0), 10.0);
            book.reject();
            assert_eq!(
                book.accounts
                    .iter()
                    .map(|a| a.units.to_bits())
                    .collect::<Vec<_>>(),
                initial_units
            );
            assert_eq!(book.finance[0].map(f64::to_bits), initial_finance);
        }
    }
}
