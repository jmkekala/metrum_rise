//! Shared fiscal arithmetic for city revenue flows.

use crate::simulation::economy::definitions::FiscalRuntimeTuning;
use crate::simulation::zoning::ZoneType;

/// Tax revenue collected by economy subsystems during one deterministic phase.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FiscalRevenue {
    /// Wage tax withheld from household gross income.
    pub income_tax: f32,
    /// Household purchase VAT collected from store shopping.
    pub household_vat: f32,
    /// Business purchase tax collected from local and OWA input freight.
    pub business_purchase_tax: f32,
    /// Daily tax collected from positive commercial and industrial budget growth.
    pub business_profit_tax: f32,
    /// One-time property tax collected from new private building construction.
    pub property_tax: f32,
}

/// Returns a non-negative tax amount for a base transaction value and authored rate.
pub(crate) fn tax_amount(base_amount: f32, rate: f32) -> f32 {
    if base_amount <= 0.0 || rate <= 0.0 {
        0.0
    } else {
        base_amount * rate.clamp(0.0, 1.0)
    }
}

/// Splits a tax-inclusive payment into seller revenue and city tax revenue.
pub(crate) fn split_gross_tax(gross_amount: f32, rate: f32) -> (f32, f32) {
    if gross_amount <= 0.0 {
        return (0.0, 0.0);
    }
    let rate = rate.clamp(0.0, 1.0);
    if rate <= 0.0 {
        return (gross_amount, 0.0);
    }
    let base_amount = gross_amount / (1.0 + rate);
    (base_amount, gross_amount - base_amount)
}

/// Computes the one-time property tax charged when private construction starts.
pub(crate) fn construction_property_tax(
    zone_type: ZoneType,
    level: u8,
    fiscal: &FiscalRuntimeTuning,
) -> f32 {
    let base = match zone_type {
        ZoneType::Residential => fiscal.residential_property_tax_base,
        ZoneType::Commercial => fiscal.commercial_property_tax_base,
        ZoneType::Industrial => fiscal.industrial_property_tax_base,
        ZoneType::Office | ZoneType::Mixed | ZoneType::None => 0.0,
    };
    if base <= 0.0 {
        return 0.0;
    }
    let level_steps = u32::from(level.saturating_sub(1));
    base * fiscal
        .property_tax_level_multiplier
        .max(1.0)
        .powi(level_steps as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_fiscal_tuning() -> FiscalRuntimeTuning {
        FiscalRuntimeTuning {
            income_tax_rate: 0.12,
            household_vat_rate: 0.08,
            business_purchase_tax_rate: 0.03,
            business_profit_tax_rate: 0.10,
            residential_property_tax_base: 250.0,
            commercial_property_tax_base: 500.0,
            industrial_property_tax_base: 750.0,
            property_tax_level_multiplier: 1.75,
        }
    }

    #[test]
    fn split_gross_tax_returns_seller_revenue_and_city_tax() {
        let (base, tax) = split_gross_tax(108.0, 0.08);

        assert!((base - 100.0).abs() < 0.001);
        assert!((tax - 8.0).abs() < 0.001);
    }

    #[test]
    fn construction_property_tax_uses_zone_base_and_level_multiplier() {
        let fiscal = test_fiscal_tuning();

        assert!(
            (construction_property_tax(ZoneType::Residential, 1, &fiscal) - 250.0).abs() < 0.001
        );
        assert!(
            (construction_property_tax(ZoneType::Commercial, 2, &fiscal) - 875.0).abs() < 0.001
        );
        assert_eq!(construction_property_tax(ZoneType::None, 3, &fiscal), 0.0);
    }
}
