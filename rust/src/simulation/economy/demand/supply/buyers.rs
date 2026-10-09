// SPDX-License-Identifier: GPL-2.0-only

//! Publishes future input purchases only after the project's own operation is financed.

use super::*;

impl SiteSupply<'_> {
    /// Parcel storage supplies the existing O(1) identity lookup. The base stays fixed for
    /// an execution epoch, including successful allocator appends during that epoch.
    pub(super) fn project_owner(&self, parcel: u64) -> Option<usize> {
        self.zoning.parcels.storage_index(ParcelId::from_raw(parcel))
            .map(|index| self.reservations.owner_base + index)
    }

    /// New purchases cannot remove any incumbent's sales. Replacement buyers need the
    /// affected-operation proof before they can use this publication path.
    pub(super) fn publish_new_buyer(&mut self, profile: &EconomyProfileRuntime) -> Result<bool, StorageRequirement> {
        let Some(owner) = self.reservations.staged_owner else { return Ok(false); };
        if self.reservations.operations[owner].is_some() { return Ok(false); }
        if !self.commit_project()? { return Ok(false); }
        if profile.inputs.is_empty() { return Ok(true); }
        if self.buyer.is_none() && self.buyer_site.is_none() { return Ok(false); }
        let operation = self.reservations.staged_operation.unwrap();

        let building = self.buyer.map(|index| &self.allocator.buildings[index]);
        let supplies = &*self.book;
        let outside = self.carrier.filter(|_| self.goods_multiplier.is_finite());
        let multiplier = self.goods_multiplier;
        let catalog = self.catalog;
        let tuning = self.tuning;
        let mut tranches = 0;
        OperatingCosts::for_plan_with_input_quotes(profile, operation.plan, operation.prices,
            |_, _| 0.0, |input| {
                let imported = crate::simulation::economy::logistics::imported_input_unit_price(
                    catalog, tuning, input.resource, multiplier);
                let complete = supplies.visit_input_sources(owner, input, imported, outside,
                    |_, _, _, _| tranches += 1);
                assert!(complete, "funded project must retain its complete input quotation");
                Default::default()
            });
        let seed = |book: &mut crate::simulation::economy::customers::business::BusinessBudgets| {
            OperatingCosts::for_plan_with_input_quotes(profile, operation.plan, operation.prices,
                |_, _| 0.0, |input| {
                    let imported = crate::simulation::economy::logistics::imported_input_unit_price(
                        catalog, tuning, input.resource, multiplier);
                    supplies.visit_input_sources(owner, input, imported, outside,
                        |source, units, goods, carrier| {
                            if let Some(slot) = book.push_input(owner, input, goods, carrier,
                                building.map_or(0.0, |building| building.inventory_units(input.resource)),
                                units, multiplier) {
                                if let Some(source) = source { book.seed_for(source, slot); }
                            }
                        });
                    Default::default()
                });
        };
        // A new buyer has no promises to reconcile, but uses the same exact account/receipt
        // preflight as a repriced buyer. Additional supplier sales cannot reduce their funding.
        self.reservations.business.seed_new_buyer_inputs(owner, self.buyer_site,
            operation.headroom, tranches, seed)?;
        Ok(true)
    }
}
