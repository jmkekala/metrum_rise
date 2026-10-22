// SPDX-License-Identifier: GPL-2.0-only

//! Exact binary rows of the census input copy (`SIM-02-I`): every field of a building,
//! household and fiscal policy as little-endian bits, so a loaded copy is the saved one bit for
//! bit. Each codec destructures its struct without `..`, so a new field fails to compile until
//! it is saved.

use godot::prelude::Vector2;

use crate::simulation::buildings::allocator::Building;
use crate::simulation::core::handles::Handle;
use crate::simulation::economy::electricity::{
    BuildingElectricityUsage, ElectricityBill, ElectricityUsage,
};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::Household;
use crate::simulation::zoning::ZoneType;

// Saved value of an absent reference (`usize::MAX`) or of an edge the save dropped.
pub(super) const NO_REF: u64 = u64::MAX;

/// Appends little-endian values to a buffer.
pub(super) struct Writer(pub(super) Vec<u8>);

impl Writer {
    pub(super) fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    pub(super) fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub(super) fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    pub(super) fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    pub(super) fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    pub(super) fn f32(&mut self, value: f32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    pub(super) fn f64(&mut self, value: f64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    // A list length; every saved list is far below `u32::MAX` items.
    pub(super) fn len(&mut self, len: usize) {
        self.u32(len as u32);
    }

    // A dense index or `usize::MAX` for none.
    pub(super) fn index(&mut self, value: usize) {
        self.u64(if value == usize::MAX {
            NO_REF
        } else {
            value as u64
        });
    }
}

/// Reads what [`Writer`] wrote, failing on truncated or overlong buffers.
pub(super) struct Reader<'a>(pub(super) &'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or("census input copy is truncated")?;
        self.0 = rest;
        Ok(*head)
    }

    pub(super) fn u8(&mut self) -> Result<u8, String> {
        self.take().map(u8::from_le_bytes)
    }

    pub(super) fn bool(&mut self) -> Result<bool, String> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("census input copy has an invalid flag".into()),
        }
    }

    pub(super) fn u16(&mut self) -> Result<u16, String> {
        self.take().map(u16::from_le_bytes)
    }

    pub(super) fn u32(&mut self) -> Result<u32, String> {
        self.take().map(u32::from_le_bytes)
    }

    pub(super) fn u64(&mut self) -> Result<u64, String> {
        self.take().map(u64::from_le_bytes)
    }

    pub(super) fn f32(&mut self) -> Result<f32, String> {
        self.take().map(f32::from_le_bytes)
    }

    pub(super) fn f64(&mut self) -> Result<f64, String> {
        self.take().map(f64::from_le_bytes)
    }

    // A list length whose items of at least `item_bytes` each fit in the rest of the buffer.
    pub(super) fn len(&mut self, item_bytes: usize) -> Result<usize, String> {
        let len = self.u32()? as usize;
        if len.saturating_mul(item_bytes) > self.0.len() {
            return Err("census input copy list overruns its buffer".into());
        }
        Ok(len)
    }

    pub(super) fn index(&mut self) -> Result<usize, String> {
        match self.u64()? {
            NO_REF => Ok(usize::MAX),
            value => usize::try_from(value).map_err(|_| "census input index overflow".into()),
        }
    }

    pub(super) fn finish(&self) -> Result<(), String> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err("census input copy has trailing bytes".into())
        }
    }
}

// Smallest encoded building and household rows, bounding list lengths before reading them.
pub(super) const MIN_BUILDING_BYTES: usize = 200;
pub(super) const MIN_HOUSEHOLD_BYTES: usize = 80;

fn put_usage(out: &mut Writer, usage: BuildingElectricityUsage) {
    let BuildingElectricityUsage {
        workplace,
        residential,
    } = usage;
    for ElectricityUsage {
        consumed_kwh,
        local_kwh,
    } in [workplace, residential]
    {
        out.f32(consumed_kwh);
        out.f32(local_kwh);
    }
}

fn read_usage(reader: &mut Reader<'_>) -> Result<BuildingElectricityUsage, String> {
    let mut usage = || -> Result<ElectricityUsage, String> {
        Ok(ElectricityUsage {
            consumed_kwh: reader.f32()?,
            local_kwh: reader.f32()?,
        })
    };
    Ok(BuildingElectricityUsage {
        workplace: usage()?,
        residential: usage()?,
    })
}

/// Writes `building`, its road edge through `edge` (the saved edge id, or [`NO_REF`]).
pub(super) fn put_building(out: &mut Writer, building: &Building, edge: &impl Fn(usize) -> u64) {
    let Building {
        center_x,
        center_y,
        support_height_m,
        width_cells,
        depth_cells,
        zone_profile_runtime_id,
        parcel_id,
        applicant_seed,
        build_generation,
        zone_type,
        facing_dir,
        frontage_t,
        side_offset,
        front_gap_m: _,
        is_deserted,
        budget_distress,
        edge_idx,
        side,
        cell_x,
        cell_y,
        occupancy,
        worker_count,
        service_funding_override,
        asset_id,
        level,
        construction_total_hours,
        construction_complete_slot,
        broken,
        economy_profile_runtime_id,
        economy_broken,
        resource_inventory,
        revenue,
        operating_budget,
        profit_tax_budget_baseline,
        last_day_profit,
        shipment_cooldown_until_slot,
        daily_owa_input_value,
        daily_local_input_value,
        daily_city_funded_input_cost,
        daily_household_sales_value,
        daily_power_service_units,
        daily_electricity,
        recent_electricity,
        hourly_economy_activity,
        daily_power_served_units,
        recent_power_service_units,
        recent_power_served_units,
        recent_household_sales_value,
        paid_workers_in_window,
        funded_job_capacity,
        unpaid_cash_wages_in_window,
        commercial_activity_floor_scale,
        funded_activity,
        service_sales_capacity,
        service_sales_profile_runtime_id,
        work_area_scale,
        pending_redevelopment,
        rezone_grace_days_remaining,
    } = building;
    for value in [*center_x, *center_y, *support_height_m] {
        out.f32(value);
    }
    for value in [*width_cells, *depth_cells, *zone_profile_runtime_id] {
        out.u16(value);
    }
    out.u64(*parcel_id);
    out.u64(*applicant_seed);
    out.u32(*build_generation);
    out.u8(*zone_type as u8);
    for value in [facing_dir.x, facing_dir.y, *frontage_t, *side_offset] {
        out.f32(value);
    }
    out.bool(*is_deserted);
    out.bool(*budget_distress);
    out.u64(if *edge_idx == usize::MAX {
        NO_REF
    } else {
        edge(*edge_idx)
    });
    out.u8(*side as u8);
    out.index(*cell_x);
    out.u16(*cell_y);
    out.u32(*occupancy);
    out.u32(*worker_count);
    out.f32(*service_funding_override);
    out.len(asset_id.len());
    out.0.extend_from_slice(asset_id.as_bytes());
    out.u8(*level);
    out.u16(*construction_total_hours);
    out.u64(*construction_complete_slot);
    out.bool(*broken);
    out.u16(*economy_profile_runtime_id);
    out.bool(*economy_broken);
    out.len(resource_inventory.len());
    for &amount in resource_inventory {
        out.f32(amount);
    }
    for value in [
        *revenue,
        *operating_budget,
        *profit_tax_budget_baseline,
        *last_day_profit,
    ] {
        out.f32(value);
    }
    out.u64(*shipment_cooldown_until_slot);
    for value in [
        *daily_owa_input_value,
        *daily_local_input_value,
        *daily_city_funded_input_cost,
        *daily_household_sales_value,
        *daily_power_service_units,
    ] {
        out.f32(value);
    }
    put_usage(out, *daily_electricity);
    put_usage(out, *recent_electricity);
    for value in [
        *hourly_economy_activity,
        *daily_power_served_units,
        *recent_power_service_units,
        *recent_power_served_units,
        *recent_household_sales_value,
    ] {
        out.f32(value);
    }
    for value in [
        *paid_workers_in_window,
        *funded_job_capacity,
        *unpaid_cash_wages_in_window,
    ] {
        out.u32(value);
    }
    for value in [
        *commercial_activity_floor_scale,
        *funded_activity,
        *service_sales_capacity,
    ] {
        out.f32(value);
    }
    out.u16(*service_sales_profile_runtime_id);
    out.f32(*work_area_scale);
    out.bool(*pending_redevelopment);
    out.u8(*rezone_grace_days_remaining);
}

/// Reads a building written by [`put_building`]; its edge is the saved edge id.
pub(super) fn read_building(reader: &mut Reader<'_>) -> Result<Building, String> {
    let [center_x, center_y, support_height_m] = [reader.f32()?, reader.f32()?, reader.f32()?];
    let [width_cells, depth_cells, zone_profile_runtime_id] =
        [reader.u16()?, reader.u16()?, reader.u16()?];
    let parcel_id = reader.u64()?;
    let applicant_seed = reader.u64()?;
    let build_generation = reader.u32()?;
    let zone_byte = reader.u8()?;
    let zone_type = ZoneType::from_u8(zone_byte);
    if zone_type as u8 != zone_byte {
        return Err("census input building has an unknown zone".into());
    }
    let facing_dir = Vector2::new(reader.f32()?, reader.f32()?);
    let [frontage_t, side_offset] = [reader.f32()?, reader.f32()?];
    let is_deserted = reader.bool()?;
    let budget_distress = reader.bool()?;
    let edge_idx = reader.index()?;
    let side = reader.u8()? as i8;
    let cell_x = reader.index()?;
    let cell_y = reader.u16()?;
    let occupancy = reader.u32()?;
    let worker_count = reader.u32()?;
    let service_funding_override = reader.f32()?;
    let asset_len = reader.len(1)?;
    let (asset, rest) = reader.0.split_at(asset_len);
    let asset_id = std::str::from_utf8(asset)
        .map_err(|_| "census input building asset is not UTF-8".to_string())?
        .to_owned();
    reader.0 = rest;
    let level = reader.u8()?;
    let construction_total_hours = reader.u16()?;
    let construction_complete_slot = reader.u64()?;
    let broken = reader.bool()?;
    let economy_profile_runtime_id = reader.u16()?;
    let economy_broken = reader.bool()?;
    let resource_inventory = (0..reader.len(4)?)
        .map(|_| reader.f32())
        .collect::<Result<_, _>>()?;
    let [
        revenue,
        operating_budget,
        profit_tax_budget_baseline,
        last_day_profit,
    ] = [reader.f32()?, reader.f32()?, reader.f32()?, reader.f32()?];
    let shipment_cooldown_until_slot = reader.u64()?;
    let [
        daily_owa_input_value,
        daily_local_input_value,
        daily_city_funded_input_cost,
        daily_household_sales_value,
        daily_power_service_units,
    ] = [
        reader.f32()?,
        reader.f32()?,
        reader.f32()?,
        reader.f32()?,
        reader.f32()?,
    ];
    let daily_electricity = read_usage(reader)?;
    let recent_electricity = read_usage(reader)?;
    let [
        hourly_economy_activity,
        daily_power_served_units,
        recent_power_service_units,
        recent_power_served_units,
        recent_household_sales_value,
    ] = [
        reader.f32()?,
        reader.f32()?,
        reader.f32()?,
        reader.f32()?,
        reader.f32()?,
    ];
    let [
        paid_workers_in_window,
        funded_job_capacity,
        unpaid_cash_wages_in_window,
    ] = [reader.u32()?, reader.u32()?, reader.u32()?];
    let [
        commercial_activity_floor_scale,
        funded_activity,
        service_sales_capacity,
    ] = [reader.f32()?, reader.f32()?, reader.f32()?];
    let service_sales_profile_runtime_id = reader.u16()?;
    let work_area_scale = reader.f32()?;
    let pending_redevelopment = reader.bool()?;
    let rezone_grace_days_remaining = reader.u8()?;
    Ok(Building {
        center_x,
        center_y,
        support_height_m,
        width_cells,
        depth_cells,
        zone_profile_runtime_id,
        parcel_id,
        applicant_seed,
        build_generation,
        zone_type,
        facing_dir,
        frontage_t,
        side_offset,
        front_gap_m: [0.0; 3],
        is_deserted,
        budget_distress,
        edge_idx,
        side,
        cell_x,
        cell_y,
        occupancy,
        worker_count,
        service_funding_override,
        asset_id,
        level,
        construction_total_hours,
        construction_complete_slot,
        broken,
        economy_profile_runtime_id,
        economy_broken,
        resource_inventory,
        revenue,
        operating_budget,
        profit_tax_budget_baseline,
        last_day_profit,
        shipment_cooldown_until_slot,
        daily_owa_input_value,
        daily_local_input_value,
        daily_city_funded_input_cost,
        daily_household_sales_value,
        daily_power_service_units,
        daily_electricity,
        recent_electricity,
        hourly_economy_activity,
        daily_power_served_units,
        recent_power_service_units,
        recent_power_served_units,
        recent_household_sales_value,
        paid_workers_in_window,
        funded_job_capacity,
        unpaid_cash_wages_in_window,
        commercial_activity_floor_scale,
        funded_activity,
        service_sales_capacity,
        service_sales_profile_runtime_id,
        work_area_scale,
        pending_redevelopment,
        rezone_grace_days_remaining,
    })
}

/// Writes `household`; building references are dense indices, which saves keep.
pub(super) fn put_household(out: &mut Writer, household: &Household) {
    let Household {
        home_building_id,
        power_arrears,
        budget,
        stock,
        member_count,
        child_count,
        adult_count,
        employed_adults,
        gross_daily_wages,
        shopping_has_car,
        elder_count,
        consumption_rate,
        stock_days,
        replenishment_state,
        cooldown_hours,
        replenishment_failure_count,
        reserved_store_building_id,
        reserved_amount,
        reserved_total_cost,
        shopping_agent,
        shopping_timeout_hours_remaining,
        replenishment_search_cursor,
        stay_failure_days,
        unhoused_days_elapsed,
        replenishment_offset_hours,
        unemployment_days_elapsed,
    } = household;
    let ElectricityBill { local, imported } = power_arrears;
    out.index(*home_building_id);
    for value in [*local, *imported, *budget, *stock] {
        out.f32(value);
    }
    for value in [*member_count, *child_count, *adult_count, *employed_adults] {
        out.u16(value);
    }
    out.f32(*gross_daily_wages);
    out.bool(*shopping_has_car);
    out.u16(*elder_count);
    out.f32(*consumption_rate);
    out.f32(*stock_days);
    out.u8(*replenishment_state);
    out.u16(*cooldown_hours);
    out.u16(*replenishment_failure_count);
    out.index(*reserved_store_building_id);
    out.f32(*reserved_amount);
    out.f32(*reserved_total_cost);
    out.u64(shopping_agent.to_bits());
    out.u16(*shopping_timeout_hours_remaining);
    for value in [
        *replenishment_search_cursor,
        *stay_failure_days,
        *unhoused_days_elapsed,
    ] {
        out.u32(value);
    }
    out.u16(*replenishment_offset_hours);
    out.u32(*unemployment_days_elapsed);
}

/// Reads a household written by [`put_household`].
pub(super) fn read_household(reader: &mut Reader<'_>) -> Result<Household, String> {
    let home_building_id = reader.index()?;
    let power_arrears = ElectricityBill {
        local: reader.f32()?,
        imported: reader.f32()?,
    };
    let [budget, stock] = [reader.f32()?, reader.f32()?];
    let [member_count, child_count, adult_count, employed_adults] =
        [reader.u16()?, reader.u16()?, reader.u16()?, reader.u16()?];
    let gross_daily_wages = reader.f32()?;
    let shopping_has_car = reader.bool()?;
    let elder_count = reader.u16()?;
    let consumption_rate = reader.f32()?;
    let stock_days = reader.f32()?;
    let replenishment_state = reader.u8()?;
    let cooldown_hours = reader.u16()?;
    let replenishment_failure_count = reader.u16()?;
    let reserved_store_building_id = reader.index()?;
    let reserved_amount = reader.f32()?;
    let reserved_total_cost = reader.f32()?;
    let shopping_agent = Handle::from_bits(reader.u64()?);
    let shopping_timeout_hours_remaining = reader.u16()?;
    let [
        replenishment_search_cursor,
        stay_failure_days,
        unhoused_days_elapsed,
    ] = [reader.u32()?, reader.u32()?, reader.u32()?];
    let replenishment_offset_hours = reader.u16()?;
    let unemployment_days_elapsed = reader.u32()?;
    Ok(Household {
        home_building_id,
        power_arrears,
        budget,
        stock,
        member_count,
        child_count,
        adult_count,
        employed_adults,
        gross_daily_wages,
        shopping_has_car,
        elder_count,
        consumption_rate,
        stock_days,
        replenishment_state,
        cooldown_hours,
        replenishment_failure_count,
        reserved_store_building_id,
        reserved_amount,
        reserved_total_cost,
        shopping_agent,
        shopping_timeout_hours_remaining,
        replenishment_search_cursor,
        stay_failure_days,
        unhoused_days_elapsed,
        replenishment_offset_hours,
        unemployment_days_elapsed,
    })
}

/// Writes every field of `policy`.
pub(super) fn put_fiscal(out: &mut Writer, policy: &CityFiscalPolicy) {
    let CityFiscalPolicy {
        unemployment_benefit_per_adult_per_day,
        unemployment_max_days,
        pension_per_elder_per_day,
        child_support_per_child_per_day,
        income_tax_rate,
        household_vat_rate,
        business_profit_tax_rate,
        residential_property_tax_per_home_per_day,
        commercial_property_tax_per_building_per_day,
        industrial_property_tax_per_building_per_day,
        property_tax_level_multiplier,
    } = policy;
    out.f32(*unemployment_benefit_per_adult_per_day);
    out.u32(*unemployment_max_days);
    for value in [
        *pension_per_elder_per_day,
        *child_support_per_child_per_day,
        *income_tax_rate,
        *household_vat_rate,
        *business_profit_tax_rate,
        *residential_property_tax_per_home_per_day,
        *commercial_property_tax_per_building_per_day,
        *industrial_property_tax_per_building_per_day,
        *property_tax_level_multiplier,
    ] {
        out.f32(value);
    }
}

/// Reads a policy written by [`put_fiscal`].
pub(super) fn read_fiscal(reader: &mut Reader<'_>) -> Result<CityFiscalPolicy, String> {
    let unemployment_benefit_per_adult_per_day = reader.f32()?;
    let unemployment_max_days = reader.u32()?;
    let mut rates = [0.0; 9];
    for rate in &mut rates {
        *rate = reader.f32()?;
    }
    let [
        pension_per_elder_per_day,
        child_support_per_child_per_day,
        income_tax_rate,
        household_vat_rate,
        business_profit_tax_rate,
        residential_property_tax_per_home_per_day,
        commercial_property_tax_per_building_per_day,
        industrial_property_tax_per_building_per_day,
        property_tax_level_multiplier,
    ] = rates;
    Ok(CityFiscalPolicy {
        unemployment_benefit_per_adult_per_day,
        unemployment_max_days,
        pension_per_elder_per_day,
        child_support_per_child_per_day,
        income_tax_rate,
        household_vat_rate,
        business_profit_tax_rate,
        residential_property_tax_per_home_per_day,
        commercial_property_tax_per_building_per_day,
        industrial_property_tax_per_building_per_day,
        property_tax_level_multiplier,
    })
}
