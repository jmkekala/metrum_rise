// SPDX-License-Identifier: GPL-2.0-only

//! Joint replacement proof over the affected input buyers and their supplying operations.

use super::*;
use crate::simulation::economy::logistics::{
    building_outputs_can_export_to_owa, imported_input_unit_price,
};

/// Retained physical door, tariff and supply handle for an accepted operation.
#[derive(Clone)]
pub(super) struct OperationSite {
    /// Current actual or prepared replacement door.
    pub(super) entrance: Option<BuildingEntrance>,
    /// Live route-cache identity, absent for a changed or unplaced door.
    pub(super) buyer: Option<usize>,
    /// Prepared buyer geometry for a changed recipe or unplaced project.
    pub(super) buyer_site: Option<usize>,
    /// Active original or projected physical supplier handle.
    pub(super) source: Option<usize>,
    /// Receiving-window multiplier already applied to local/import quotes.
    pub(super) goods_multiplier: f32,
    /// Executable outside sales tariff; zero means no outside buyers.
    pub(super) exports: f32,
    /// Whether reserved future staffing may advertise the whole plan's input demand.
    pub(super) future: bool,
    /// Treasury-backed inputs use the existing public-service funding contract.
    pub(super) public: bool,
}

/// Reused sparse affected set; first snapshots survive until the full proposal commits.
#[derive(Default)]
pub(super) struct ReplacementProof {
    affected: Vec<usize>,
    marked: Vec<bool>,
    before: Vec<([f64; 2], [f64; 2])>,
    snapshots: usize,
    buyers: Vec<usize>,
    buyer_marked: Vec<bool>,
    repairs: Vec<usize>,
    repair_marked: Vec<bool>,
}

impl ReplacementProof {
    /// Sizes owner-indexed marks once per pass, never inside a candidate quote.
    pub(super) fn prepare(&mut self, owners: usize) {
        self.clear();
        // Owner rows follow the city, with spare capacity (`SIM-02-K`).
        use crate::utils::storage::{reserve_rows, resize_rows};
        resize_rows(&mut self.marked, owners, false);
        resize_rows(&mut self.before, owners, ([0.0; 2], [0.0; 2]));
        resize_rows(&mut self.buyer_marked, owners, false);
        resize_rows(&mut self.repair_marked, owners, false);
        reserve_rows(&mut self.affected, owners);
        reserve_rows(&mut self.buyers, owners);
        reserve_rows(&mut self.repairs, owners);
    }

    /// Resets only the owners touched by the preceding proposal.
    pub(super) fn clear(&mut self) {
        for owner in self.affected.drain(..) {
            self.marked[owner] = false;
        }
        for owner in self.buyers.drain(..) {
            self.buyer_marked[owner] = false;
        }
        for owner in self.repairs.drain(..) {
            self.repair_marked[owner] = false;
        }
        self.snapshots = 0;
    }

    fn touch(&mut self, owner: usize) {
        if !self.marked[owner] {
            self.marked[owner] = true;
            self.affected.push(owner);
        }
    }

    fn buyer(&mut self, owner: usize, repair: bool) {
        self.touch(owner);
        if !self.buyer_marked[owner] {
            self.buyer_marked[owner] = true;
            self.buyers.push(owner);
        }
        if repair && !self.repair_marked[owner] {
            self.repair_marked[owner] = true;
            self.repairs.push(owner);
        }
    }
}

impl ProjectReservations {
    /// Captures live door/cargo metadata beside the same retained funding census.
    pub(super) fn reset_sites(
        &mut self,
        funding: &crate::simulation::economy::hiring::JobFunding,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        minute: u16,
    ) {
        self.sites[..allocator.buildings.len()]
            .par_iter_mut()
            .enumerate()
            .for_each(|(owner, site)| {
                *site = Self::site_row(allocator, catalog, tuning, minute, owner);
            });
        self.prepare_site_scratch(funding);
    }

    /// Live building `owner`'s door and cargo metadata at `minute`'s price phase. O(1).
    pub(super) fn site_row(
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        minute: u16,
        owner: usize,
    ) -> Option<OperationSite> {
        let building = &allocator.buildings[owner];
        catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .map(|profile| OperationSite {
                entrance: allocator.entrances.get(owner).cloned(),
                buyer: Some(owner),
                buyer_site: None,
                source: Some(owner),
                goods_multiplier: freight_goods_multiplier(
                    profile,
                    building.zone_type,
                    tuning,
                    minute,
                ),
                exports: if building_outputs_can_export_to_owa(building, profile) {
                    tuning.owa_export_price_multiplier
                } else {
                    0.0
                },
                future: false,
                public: allocator.is_city_service_building(building),
            })
    }

    /// The pass's freight capital and empty site staging. O(B) copy.
    pub(super) fn prepare_site_scratch(
        &mut self,
        funding: &crate::simulation::economy::hiring::JobFunding,
    ) {
        self.freight_capital.clone_from(&funding.freight_capital);
        self.staged_site = None;
        self.replacement_proof.clear();
    }

    /// Publishes affected headroom only after workforce and placement acceptance.
    pub(super) fn accept_replacement_finance(&mut self) {
        for &owner in &self.replacement_proof.affected {
            if let Some(operation) = &mut self.operations[owner] {
                operation.headroom = self.business.financing(owner);
                // The next pass's start derives this row again (`SIM-02-K`).
                self.touched_live.push(owner);
            }
        }
        self.replacement_proof.clear();
    }
}

impl SiteSupply<'_> {
    /// Stages the candidate's prepared geometry with its operation, without heap allocation.
    pub(super) fn retain_site(&mut self) {
        self.reservations.staged_site = Some(OperationSite {
            entrance: self.entrance.clone(),
            buyer: self.buyer,
            buyer_site: self.buyer_site,
            source: self.replacement_source,
            goods_multiplier: self.goods_multiplier,
            exports: self.exports,
            future: true,
            public: self.buyer.is_some_and(|owner| {
                self.allocator
                    .is_city_service_building(&self.allocator.buildings[owner])
            }),
        });
    }

    fn retained_operation(&self, owner: usize) -> Option<(FundedOperation, &OperationSite)> {
        if self.reservations.staged_owner == Some(owner)
            && let Some(operation) = self.reservations.staged_operation
            && let Some(site) = &self.reservations.staged_site
        {
            Some((operation, site))
        } else {
            Some((
                self.reservations.operations[owner]?,
                self.reservations.sites[owner].as_ref()?,
            ))
        }
    }

    // Seller-only changes cannot alter input costs. Keep these two output contributions
    // separate so their exact delta can update an otherwise unchanged financing proof.
    fn output_value(&self, owner: usize) -> Option<[f64; 2]> {
        let (operation, site) = self.retained_operation(owner)?;
        let profile = self.catalog.profile_by_runtime_id(operation.profile)?;
        let mut value = [0.0; 2];
        for port in &profile.outputs {
            let resource = port.resource_runtime_id;
            let full = profile.net_output_units_per_day(port) * operation.plan.area;
            let local = |limit: f64| {
                let household = self
                    .reservations
                    .customers
                    .reserved_units(owner, resource)
                    .min(limit);
                let business =
                    self.reservations
                        .business
                        .sales(owner, resource, (limit - household).max(0.0));
                (
                    household + business.units,
                    household * f64::from(profile.unit_price_currency) + business.revenue,
                )
            };
            let export_price = if site.exports > 0.0 && operation.prices.carrier_quote.is_finite() {
                self.catalog
                    .unit_price_for_resource(resource)
                    .unwrap_or(0.0)
                    * site.exports
            } else {
                0.0
            };
            let units = full * operation.plan.activity;
            let (sold, revenue) = local(f64::from(units));
            let exported = if export_price > 0.0 {
                (f64::from(units) - sold).max(0.0)
            } else {
                0.0
            };
            let freight = export_freight(profile, port, units, exported as f32, operation.prices);
            value[0] += revenue + exported * f64::from(export_price) - freight;
            value[1] -= freight;
            if owner < self.reservations.owner_base
                && profile.input_port(resource).is_none()
                && full > 0.0
            {
                let (sold, revenue) = local(f64::from(full));
                let price = (revenue + (f64::from(full) - sold).max(0.0) * f64::from(export_price))
                    / f64::from(full);
                value[1] +=
                    f64::from(self.allocator.buildings[owner].inventory_units(resource)) * price;
            }
        }
        Some(value)
    }

    fn snapshot_affected(&mut self) {
        while self.reservations.replacement_proof.snapshots
            < self.reservations.replacement_proof.affected.len()
        {
            let index = self.reservations.replacement_proof.snapshots;
            let owner = self.reservations.replacement_proof.affected[index];
            self.reservations.replacement_proof.before[owner] = (
                self.reservations.business.financing(owner),
                self.output_value(owner).unwrap_or([0.0; 2]),
            );
            self.reservations.replacement_proof.snapshots += 1;
        }
    }

    /// Withdraws the old recipe's output and purchases before pricing its replacement.
    pub(super) fn withdraw_replacement(&mut self, owner: usize) {
        let proof = &mut self.reservations.replacement_proof;
        proof.touch(owner);
        self.reservations
            .business
            .visit_suppliers(owner, |seller| proof.touch(seller));
        self.reservations
            .business
            .visit_customers(owner, |buyer, _, _, _| proof.buyer(buyer, false));
        let source = self.reservations.sites[owner]
            .as_ref()
            .and_then(|site| site.source);
        if let Some(source) = source {
            self.book
                .visit_dependents(source, |buyer, _, _| proof.buyer(buyer, true));
        }
        self.snapshot_affected();
        if let Some(source) = source {
            self.book.replace_source_output(
                source,
                1.0,
                0.0,
                |_| 0.0,
                self.allocator,
                self.catalog,
                self.graph,
            );
        }
        self.reservations
            .business
            .withdraw_buyer(owner, |_, _, _, _| {});
    }

    fn physical_headroom(&self, owner: usize) -> Option<[f64; 2]> {
        let (operation, site) = self.retained_operation(owner)?;
        let profile = self.catalog.profile_by_runtime_id(operation.profile)?;
        let building =
            (owner < self.reservations.owner_base).then(|| &self.allocator.buildings[owner]);
        let mut capital =
            building.map_or_else(
                || {
                    f64::from(profile.zoned_startup_budget(
                        self.catalog,
                        self.tuning.owa_import_price_multiplier,
                    ))
                },
                |building| {
                    f64::from(building.operating_budget)
                        + self
                            .reservations
                            .freight_capital
                            .get(owner)
                            .copied()
                            .unwrap_or(0.0)
                },
            );
        let outside = operation
            .prices
            .carrier_quote
            .is_finite()
            .then_some(operation.prices.carrier_quote)
            .filter(|_| site.goods_multiplier.is_finite());
        let costs = OperatingCosts::for_plan_with_input_quotes(
            profile,
            operation.plan,
            operation.prices,
            |_, _| 0.0,
            |input| {
                let imported = imported_input_unit_price(
                    self.catalog,
                    self.tuning,
                    input.resource,
                    site.goods_multiplier,
                );
                let quote = self.book.owner_input_costs(owner, input, imported, outside);
                if let Some(building) = building {
                    let price = if input.daily_units > 0.0 {
                        quote.goods / f64::from(input.daily_units)
                    } else {
                        f64::from(imported)
                    };
                    capital += f64::from(building.inventory_units(input.resource)) * price;
                }
                quote
            },
        );
        if !costs.total().is_finite() || !costs.working_capital.is_finite() {
            return None;
        }
        if site.public {
            return Some([f64::INFINITY; 2]);
        }
        let output = self.output_value(owner)?;
        Some([
            output[0] - costs.total(),
            capital + output[1] - costs.working_capital,
        ])
    }

    fn repair_input_gap(&mut self, owner: usize) -> Result<bool, StorageRequirement> {
        let Some((operation, site)) = self.retained_operation(owner) else {
            return Ok(false);
        };
        let site = site.clone();
        let Some(entrance) = &site.entrance else {
            return Ok(false);
        };
        let Some(profile) = self.catalog.profile_by_runtime_id(operation.profile) else {
            return Ok(false);
        };
        let outside = operation
            .prices
            .carrier_quote
            .is_finite()
            .then_some(operation.prices.carrier_quote)
            .filter(|_| site.goods_multiplier.is_finite());
        let costs = OperatingCosts::for_plan_with_input_quotes(
            profile,
            operation.plan,
            operation.prices,
            |_, _| 0.0,
            |input| {
                self.logistics.quote_missing_recurring_input(
                    self.book,
                    input,
                    owner,
                    site.buyer,
                    entrance,
                    outside,
                    self.allocator,
                    self.catalog,
                    self.tuning,
                    self.network,
                    self.graph,
                    site.goods_multiplier,
                )
            },
        );
        if !costs.total().is_finite() || !costs.working_capital.is_finite() {
            return Ok(false);
        }
        self.book
            .check_extension_capacity()
            .map_err(StorageRequirement::Inputs)?;
        self.book.extend_inputs_for(owner);
        Ok(true)
    }

    fn reprice_buyer(&mut self, owner: usize) -> Result<bool, [usize; 2]> {
        let Some((operation, site)) = self.retained_operation(owner) else {
            return Ok(false);
        };
        let site = site.clone();
        let Some(profile) = self.catalog.profile_by_runtime_id(operation.profile) else {
            return Ok(false);
        };
        let Some(headroom) = self.physical_headroom(owner) else {
            return Ok(false);
        };
        let building =
            (owner < self.reservations.owner_base).then(|| &self.allocator.buildings[owner]);
        let staffed = if site.future {
            1.0
        } else {
            let physical = building.map_or(0, |building| {
                physical_worker_capacity_for_profile(building, profile)
            });
            let activity = building
                .map_or(0.0, |building| {
                    building.worker_count as f32 / physical.max(1) as f32
                })
                .min(operation.plan.activity);
            if operation.plan.activity > 0.0 {
                activity / operation.plan.activity
            } else {
                0.0
            }
        };
        let outside = operation
            .prices
            .carrier_quote
            .is_finite()
            .then_some(operation.prices.carrier_quote)
            .filter(|_| site.goods_multiplier.is_finite());
        let proof = &mut self.reservations.replacement_proof;
        self.reservations
            .business
            .visit_suppliers(owner, |seller| proof.touch(seller));
        let mut tranches = 0;
        let mut complete = true;
        OperatingCosts::for_plan_with_input_quotes(
            profile,
            operation.plan,
            operation.prices,
            |_, _| 0.0,
            |input| {
                let imported = imported_input_unit_price(
                    self.catalog,
                    self.tuning,
                    input.resource,
                    site.goods_multiplier,
                );
                complete &= self.book.visit_input_sources(
                    owner,
                    InputRequirement {
                        daily_units: input.daily_units * staffed,
                        ..input
                    },
                    imported,
                    outside,
                    |source, _, _, _| {
                        tranches += 1;
                        if let Some(source) = source {
                            proof.touch(source);
                        }
                    },
                );
                Default::default()
            },
        );
        if !complete {
            return Ok(false);
        }
        self.snapshot_affected();
        // Material receipts and account mutations borrow distinct stores. Keep the seed's
        // immutable inputs outside the mutable market borrow; no book clone is required.
        let supplies = &*self.book;
        let catalog = self.catalog;
        let tuning = self.tuning;
        let result = self.reservations.business.reprice_buyer_inputs(
            owner,
            site.buyer_site,
            headroom,
            tranches,
            |book| {
                OperatingCosts::for_plan_with_input_quotes(
                    profile,
                    operation.plan,
                    operation.prices,
                    |_, _| 0.0,
                    |input| {
                        let imported = imported_input_unit_price(
                            catalog,
                            tuning,
                            input.resource,
                            site.goods_multiplier,
                        );
                        supplies.visit_input_sources(
                            owner,
                            InputRequirement {
                                daily_units: input.daily_units * staffed,
                                ..input
                            },
                            imported,
                            outside,
                            |source, units, goods, carrier| {
                                if let Some(slot) = book.push_input(
                                    owner,
                                    input,
                                    goods,
                                    carrier,
                                    building.map_or(0.0, |building| {
                                        building.inventory_units(input.resource)
                                    }),
                                    units,
                                    site.goods_multiplier,
                                ) && let Some(source) = source
                                {
                                    book.seed_for(source, slot);
                                }
                            },
                        );
                        Default::default()
                    },
                );
            },
            |_, _, _, _| {},
        )?;
        if !result {
            return Ok(false);
        }
        self.reservations.replacement_proof.before[owner] = (
            self.reservations.business.financing(owner),
            self.output_value(owner).unwrap(),
        );
        Ok(true)
    }

    /// Proves changed output, repaired inputs and all affected financing in one outer journal.
    pub(super) fn fund_replacement(
        &mut self,
        profile: &EconomyProfileRuntime,
    ) -> Result<bool, StorageRequirement> {
        let owner = self.replacement.unwrap();
        if !profile.inputs.is_empty() && self.buyer_site.is_none() {
            return Ok(false);
        }
        if !self.commit_project()? {
            return Ok(false);
        }
        let proof = &mut self.reservations.replacement_proof;
        self.reservations
            .business
            .visit_customers(owner, |buyer, _, _, _| proof.buyer(buyer, false));
        // Every buyer keeps its other suppliers. Remove the changed source before the new
        // staffed output is delivered; no old and new production can coexist in the quote.
        if let Some(source) = self.reservations.sites[owner]
            .as_ref()
            .and_then(|site| site.source)
        {
            for &buyer in &proof.repairs {
                self.book
                    .release_source_from_owner(buyer, source, self.allocator, self.graph);
            }
        }
        if let Some(source) = self.replacement_source {
            let physical =
                physical_worker_capacity_for_profile(&self.allocator.buildings[owner], profile)
                    .max(1);
            let activity = (self.allocator.buildings[owner].worker_count as f32 / physical as f32)
                .min(self.plan.activity);
            self.book.replace_source_output(
                source,
                self.plan.area,
                activity,
                |resource| self.reservations.customers.reserved_units(owner, resource),
                self.allocator,
                self.catalog,
                self.graph,
            );
            let book = &mut self.book;
            let mut result = Ok(());
            self.reservations
                .business
                .visit_customers(owner, |buyer, resource, units, quote| {
                    if result.is_ok() {
                        result = book.commit_customer_delivery(
                            source,
                            buyer,
                            resource,
                            units,
                            quote,
                            self.allocator,
                            self.catalog,
                            self.graph,
                        );
                    }
                });
            result.map_err(StorageRequirement::Inputs)?;
        }
        self.reservations.replacement_proof.repairs.sort_unstable();
        for index in 0..self.reservations.replacement_proof.repairs.len() {
            let buyer = self.reservations.replacement_proof.repairs[index];
            if !self.repair_input_gap(buyer)? {
                return Ok(false);
            }
        }
        if !self.reprice_buyer(owner)? {
            return Ok(false);
        }
        self.reservations.replacement_proof.buyers.sort_unstable();
        for index in 0..self.reservations.replacement_proof.buyers.len() {
            let buyer = self.reservations.replacement_proof.buyers[index];
            if buyer != owner && !self.reprice_buyer(buyer)? {
                return Ok(false);
            }
        }
        for index in 0..self.reservations.replacement_proof.affected.len() {
            let affected = self.reservations.replacement_proof.affected[index];
            let Some(output) = self.output_value(affected) else {
                return Ok(false);
            };
            let (finance, before) = self.reservations.replacement_proof.before[affected];
            let after = [
                finance[0] + (output[0] - before[0]),
                finance[1] + (output[1] - before[1]),
            ];
            if !(after[0] > 0.0 && after[1] >= 0.0) {
                return Ok(false);
            }
            self.reservations.business.set_financing(affected, after);
        }
        self.reservations
            .staged_operation
            .as_mut()
            .unwrap()
            .headroom = self.reservations.business.financing(owner);
        Ok(true)
    }
}
