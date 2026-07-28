//! Shared fiscal policy and arithmetic for city revenue and transfer flows.

use crate::simulation::economy::definitions::{RuntimeEconomyTuning, load_runtime_economy_tuning};
use crate::simulation::zoning::ZoneType;

/// Stable policy id for daily unemployment benefit paid per unemployed adult.
pub(crate) const POLICY_UNEMPLOYMENT_BENEFIT: &str = "unemployment_benefit";
/// Stable policy id for maximum unemployment benefit duration.
pub(crate) const POLICY_UNEMPLOYMENT_MAX_DAYS: &str = "unemployment_max_days";
/// Stable policy id for daily pension paid per elder.
pub(crate) const POLICY_PENSION: &str = "pension";
/// Stable policy id for daily child support paid per child.
pub(crate) const POLICY_CHILD_SUPPORT: &str = "child_support";
/// Stable policy id for wage income tax.
pub(crate) const POLICY_INCOME_TAX: &str = "income_tax";
/// Stable policy id for household purchase VAT.
pub(crate) const POLICY_HOUSEHOLD_VAT: &str = "household_vat";
/// Stable policy id for business input purchase tax.
pub(crate) const POLICY_BUSINESS_PURCHASE_TAX: &str = "business_purchase_tax";
/// Stable policy id for positive business-budget growth tax.
pub(crate) const POLICY_BUSINESS_PROFIT_TAX: &str = "business_profit_tax";
/// Stable policy id for residential construction property tax base.
pub(crate) const POLICY_RESIDENTIAL_PROPERTY_TAX: &str = "residential_property_tax";
/// Stable policy id for commercial construction property tax base.
pub(crate) const POLICY_COMMERCIAL_PROPERTY_TAX: &str = "commercial_property_tax";
/// Stable policy id for industrial construction property tax base.
pub(crate) const POLICY_INDUSTRIAL_PROPERTY_TAX: &str = "industrial_property_tax";
/// Stable policy id for per-level property-tax multiplier.
pub(crate) const POLICY_PROPERTY_TAX_LEVEL_MULTIPLIER: &str = "property_tax_level_multiplier";

const TRANSFER_MIN: f32 = 0.0;
const TRANSFER_MAX: f32 = 200.0;
const UNEMPLOYMENT_DAYS_MIN: f32 = 0.0;
const UNEMPLOYMENT_DAYS_MAX: f32 = 365.0;
const INCOME_TAX_MAX: f32 = 0.75;
const VAT_MAX: f32 = 0.50;
const BUSINESS_PROFIT_TAX_MAX: f32 = 0.75;
const PROPERTY_TAX_BASE_MIN: f32 = 0.0;
const PROPERTY_TAX_BASE_MAX: f32 = 5_000.0;
const PROPERTY_TAX_MULTIPLIER_MIN: f32 = 1.0;
const PROPERTY_TAX_MULTIPLIER_MAX: f32 = 5.0;

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

/// Player-controlled fiscal policy used by live economy systems.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CityFiscalPolicy {
    /// Currency paid per unemployed adult per day while benefit duration remains.
    pub(crate) unemployment_benefit_per_adult_per_day: f32,
    /// Maximum settled days an unemployed household may receive unemployment benefit.
    pub(crate) unemployment_max_days: u32,
    /// Currency paid per elder resident per day.
    pub(crate) pension_per_elder_per_day: f32,
    /// Currency paid per child resident per day.
    pub(crate) child_support_per_child_per_day: f32,
    /// Fraction of gross daily wages withheld before household income is received.
    pub(crate) income_tax_rate: f32,
    /// Fraction added to household store purchases and remitted to the city at pickup.
    pub(crate) household_vat_rate: f32,
    /// Fraction added to business input purchases and remitted to the city on delivery.
    pub(crate) business_purchase_tax_rate: f32,
    /// Fraction of positive daily business operating-budget growth remitted to the city.
    pub(crate) business_profit_tax_rate: f32,
    /// One-time tax charged when residential private construction starts.
    pub(crate) residential_property_tax_base: f32,
    /// One-time tax charged when commercial private construction starts.
    pub(crate) commercial_property_tax_base: f32,
    /// One-time tax charged when industrial private construction starts.
    pub(crate) industrial_property_tax_base: f32,
    /// Per-level multiplier applied to property tax above level 1.
    pub(crate) property_tax_level_multiplier: f32,
}

impl CityFiscalPolicy {
    /// Builds live policy defaults from authored economy tuning.
    pub(crate) fn from_runtime_tuning(tuning: &RuntimeEconomyTuning) -> Self {
        Self {
            unemployment_benefit_per_adult_per_day: tuning
                .unemployment_daily_benefit_per_member
                .clamp(TRANSFER_MIN, TRANSFER_MAX),
            unemployment_max_days: clamp_days(tuning.unemployment_max_days as f32),
            pension_per_elder_per_day: tuning
                .pension_daily_benefit_per_elder
                .clamp(TRANSFER_MIN, TRANSFER_MAX),
            child_support_per_child_per_day: tuning
                .child_support_daily_benefit_per_child
                .clamp(TRANSFER_MIN, TRANSFER_MAX),
            income_tax_rate: tuning.fiscal.income_tax_rate.clamp(0.0, INCOME_TAX_MAX),
            household_vat_rate: tuning.fiscal.household_vat_rate.clamp(0.0, VAT_MAX),
            business_purchase_tax_rate: tuning
                .fiscal
                .business_purchase_tax_rate
                .clamp(0.0, VAT_MAX),
            business_profit_tax_rate: tuning
                .fiscal
                .business_profit_tax_rate
                .clamp(0.0, BUSINESS_PROFIT_TAX_MAX),
            residential_property_tax_base: tuning
                .fiscal
                .residential_property_tax_base
                .clamp(PROPERTY_TAX_BASE_MIN, PROPERTY_TAX_BASE_MAX),
            commercial_property_tax_base: tuning
                .fiscal
                .commercial_property_tax_base
                .clamp(PROPERTY_TAX_BASE_MIN, PROPERTY_TAX_BASE_MAX),
            industrial_property_tax_base: tuning
                .fiscal
                .industrial_property_tax_base
                .clamp(PROPERTY_TAX_BASE_MIN, PROPERTY_TAX_BASE_MAX),
            property_tax_level_multiplier: tuning
                .fiscal
                .property_tax_level_multiplier
                .clamp(PROPERTY_TAX_MULTIPLIER_MIN, PROPERTY_TAX_MULTIPLIER_MAX),
        }
    }

    /// Applies one UI/API policy value after clamping to the supported range.
    pub(crate) fn set_value(&mut self, policy_id: &str, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match policy_id {
            POLICY_UNEMPLOYMENT_BENEFIT => {
                self.unemployment_benefit_per_adult_per_day =
                    value.clamp(TRANSFER_MIN, TRANSFER_MAX);
            }
            POLICY_UNEMPLOYMENT_MAX_DAYS => {
                self.unemployment_max_days = clamp_days(value);
            }
            POLICY_PENSION => {
                self.pension_per_elder_per_day = value.clamp(TRANSFER_MIN, TRANSFER_MAX);
            }
            POLICY_CHILD_SUPPORT => {
                self.child_support_per_child_per_day = value.clamp(TRANSFER_MIN, TRANSFER_MAX);
            }
            POLICY_INCOME_TAX => {
                self.income_tax_rate = value.clamp(0.0, INCOME_TAX_MAX);
            }
            POLICY_HOUSEHOLD_VAT => {
                self.household_vat_rate = value.clamp(0.0, VAT_MAX);
            }
            POLICY_BUSINESS_PURCHASE_TAX => {
                self.business_purchase_tax_rate = value.clamp(0.0, VAT_MAX);
            }
            POLICY_BUSINESS_PROFIT_TAX => {
                self.business_profit_tax_rate = value.clamp(0.0, BUSINESS_PROFIT_TAX_MAX);
            }
            POLICY_RESIDENTIAL_PROPERTY_TAX => {
                self.residential_property_tax_base =
                    value.clamp(PROPERTY_TAX_BASE_MIN, PROPERTY_TAX_BASE_MAX);
            }
            POLICY_COMMERCIAL_PROPERTY_TAX => {
                self.commercial_property_tax_base =
                    value.clamp(PROPERTY_TAX_BASE_MIN, PROPERTY_TAX_BASE_MAX);
            }
            POLICY_INDUSTRIAL_PROPERTY_TAX => {
                self.industrial_property_tax_base =
                    value.clamp(PROPERTY_TAX_BASE_MIN, PROPERTY_TAX_BASE_MAX);
            }
            POLICY_PROPERTY_TAX_LEVEL_MULTIPLIER => {
                self.property_tax_level_multiplier =
                    value.clamp(PROPERTY_TAX_MULTIPLIER_MIN, PROPERTY_TAX_MULTIPLIER_MAX);
            }
            _ => return false,
        }
        true
    }

    /// Returns one current control descriptor by stable policy id.
    pub(crate) fn control(self, policy_id: &str) -> Option<FiscalPolicyControl> {
        self.controls()
            .into_iter()
            .find(|control| control.id == policy_id)
    }

    /// Returns stable UI control metadata for every live fiscal policy field.
    pub(crate) fn controls(self) -> [FiscalPolicyControl; 12] {
        [
            FiscalPolicyControl::new(
                POLICY_UNEMPLOYMENT_BENEFIT,
                "Unemployment benefit",
                "Transfers",
                FiscalPolicyUnit::CurrencyPerDay,
                self.unemployment_benefit_per_adult_per_day,
                TRANSFER_MIN,
                TRANSFER_MAX,
                1.0,
            ),
            FiscalPolicyControl::new(
                POLICY_UNEMPLOYMENT_MAX_DAYS,
                "Unemployment duration",
                "Transfers",
                FiscalPolicyUnit::Days,
                self.unemployment_max_days as f32,
                UNEMPLOYMENT_DAYS_MIN,
                UNEMPLOYMENT_DAYS_MAX,
                1.0,
            ),
            FiscalPolicyControl::new(
                POLICY_PENSION,
                "Pension",
                "Transfers",
                FiscalPolicyUnit::CurrencyPerDay,
                self.pension_per_elder_per_day,
                TRANSFER_MIN,
                TRANSFER_MAX,
                1.0,
            ),
            FiscalPolicyControl::new(
                POLICY_CHILD_SUPPORT,
                "Child support",
                "Transfers",
                FiscalPolicyUnit::CurrencyPerDay,
                self.child_support_per_child_per_day,
                TRANSFER_MIN,
                TRANSFER_MAX,
                1.0,
            ),
            FiscalPolicyControl::new(
                POLICY_INCOME_TAX,
                "Income tax",
                "Taxes",
                FiscalPolicyUnit::Percent,
                self.income_tax_rate,
                0.0,
                INCOME_TAX_MAX,
                0.01,
            ),
            FiscalPolicyControl::new(
                POLICY_HOUSEHOLD_VAT,
                "Household VAT",
                "Taxes",
                FiscalPolicyUnit::Percent,
                self.household_vat_rate,
                0.0,
                VAT_MAX,
                0.01,
            ),
            FiscalPolicyControl::new(
                POLICY_BUSINESS_PURCHASE_TAX,
                "Business purchase tax",
                "Taxes",
                FiscalPolicyUnit::Percent,
                self.business_purchase_tax_rate,
                0.0,
                VAT_MAX,
                0.01,
            ),
            FiscalPolicyControl::new(
                POLICY_BUSINESS_PROFIT_TAX,
                "Business profit tax",
                "Taxes",
                FiscalPolicyUnit::Percent,
                self.business_profit_tax_rate,
                0.0,
                BUSINESS_PROFIT_TAX_MAX,
                0.01,
            ),
            FiscalPolicyControl::new(
                POLICY_RESIDENTIAL_PROPERTY_TAX,
                "Residential property tax",
                "Construction",
                FiscalPolicyUnit::Currency,
                self.residential_property_tax_base,
                PROPERTY_TAX_BASE_MIN,
                PROPERTY_TAX_BASE_MAX,
                25.0,
            ),
            FiscalPolicyControl::new(
                POLICY_COMMERCIAL_PROPERTY_TAX,
                "Commercial property tax",
                "Construction",
                FiscalPolicyUnit::Currency,
                self.commercial_property_tax_base,
                PROPERTY_TAX_BASE_MIN,
                PROPERTY_TAX_BASE_MAX,
                25.0,
            ),
            FiscalPolicyControl::new(
                POLICY_INDUSTRIAL_PROPERTY_TAX,
                "Industrial property tax",
                "Construction",
                FiscalPolicyUnit::Currency,
                self.industrial_property_tax_base,
                PROPERTY_TAX_BASE_MIN,
                PROPERTY_TAX_BASE_MAX,
                25.0,
            ),
            FiscalPolicyControl::new(
                POLICY_PROPERTY_TAX_LEVEL_MULTIPLIER,
                "Property level multiplier",
                "Construction",
                FiscalPolicyUnit::Multiplier,
                self.property_tax_level_multiplier,
                PROPERTY_TAX_MULTIPLIER_MIN,
                PROPERTY_TAX_MULTIPLIER_MAX,
                0.05,
            ),
        ]
    }
}

impl Default for CityFiscalPolicy {
    fn default() -> Self {
        load_runtime_economy_tuning()
            .map(|tuning| Self::from_runtime_tuning(tuning.as_ref()))
            .unwrap_or_else(|_| Self::fallback())
    }
}

impl CityFiscalPolicy {
    fn fallback() -> Self {
        Self {
            unemployment_benefit_per_adult_per_day: 30.0,
            unemployment_max_days: 14,
            pension_per_elder_per_day: 30.0,
            child_support_per_child_per_day: 10.0,
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
}

/// Presentation unit for one fiscal policy control.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FiscalPolicyUnit {
    /// Currency amount paid every day.
    CurrencyPerDay,
    /// Fraction shown as a percentage in UI.
    Percent,
    /// Whole operational days.
    Days,
    /// One-time currency amount.
    Currency,
    /// Unitless multiplier.
    Multiplier,
}

impl FiscalPolicyUnit {
    /// Returns the stable UI token for this unit.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CurrencyPerDay => "currency_per_day",
            Self::Percent => "percent",
            Self::Days => "days",
            Self::Currency => "currency",
            Self::Multiplier => "multiplier",
        }
    }
}

/// One slider/control descriptor exposed to Godot for the policy tab.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FiscalPolicyControl {
    /// Stable policy id accepted by [`CityFiscalPolicy::set_value`].
    pub(crate) id: &'static str,
    /// Human-facing control label.
    pub(crate) label: &'static str,
    /// Human-facing section label.
    pub(crate) group: &'static str,
    /// Presentation unit token.
    pub(crate) unit: FiscalPolicyUnit,
    /// Current clamped value.
    pub(crate) value: f32,
    /// Minimum slider/API value.
    pub(crate) min: f32,
    /// Maximum slider/API value.
    pub(crate) max: f32,
    /// Slider step size.
    pub(crate) step: f32,
}

impl FiscalPolicyControl {
    fn new(
        id: &'static str,
        label: &'static str,
        group: &'static str,
        unit: FiscalPolicyUnit,
        value: f32,
        min: f32,
        max: f32,
        step: f32,
    ) -> Self {
        Self {
            id,
            label,
            group,
            unit,
            value,
            min,
            max,
            step,
        }
    }
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
    fiscal: &CityFiscalPolicy,
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

fn clamp_days(value: f32) -> u32 {
    value
        .clamp(UNEMPLOYMENT_DAYS_MIN, UNEMPLOYMENT_DAYS_MAX)
        .round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_fiscal_policy() -> CityFiscalPolicy {
        CityFiscalPolicy {
            residential_property_tax_base: 250.0,
            commercial_property_tax_base: 500.0,
            industrial_property_tax_base: 750.0,
            property_tax_level_multiplier: 1.75,
            ..CityFiscalPolicy::fallback()
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
        let fiscal = test_fiscal_policy();

        assert!(
            (construction_property_tax(ZoneType::Residential, 1, &fiscal) - 250.0).abs() < 0.001
        );
        assert!(
            (construction_property_tax(ZoneType::Commercial, 2, &fiscal) - 875.0).abs() < 0.001
        );
        assert_eq!(construction_property_tax(ZoneType::None, 3, &fiscal), 0.0);
    }

    #[test]
    fn policy_value_setter_clamps_api_values() {
        let mut policy = CityFiscalPolicy::fallback();

        assert!(policy.set_value(POLICY_INCOME_TAX, 10.0));
        assert!((policy.income_tax_rate - INCOME_TAX_MAX).abs() < f32::EPSILON);
        assert!(policy.set_value(POLICY_UNEMPLOYMENT_MAX_DAYS, -5.0));
        assert_eq!(policy.unemployment_max_days, 0);
        assert!(policy.set_value(POLICY_PENSION, 500.0));
        assert!((policy.pension_per_elder_per_day - TRANSFER_MAX).abs() < f32::EPSILON);
        assert!(!policy.set_value("unknown", 1.0));
        assert!(!policy.set_value(POLICY_CHILD_SUPPORT, f32::NAN));
    }
}
