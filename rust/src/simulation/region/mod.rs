// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: mod.rs
//  script_path: rust/src/simulation/region/mod.rs
//  module_name: region
//  version: 0.1.0
//  description: The regional tier, as types only. Scaffold: nothing ticks
//  kind: module
//  spec: docs/economy.md
//  internal_dependencies: []
//  external_dependencies: []
//  features: [funding-scope, funding-stage, city-ledger, region-ledger]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Regional tier: cities as data points inside a shared pool.
//!
//! Scaffold only. The types below record the funding model `economy.md`
//! specifies so the shape is settled before anything depends on it. Nothing
//! ticks, nothing is saved, and no system reads these yet. Per-city statistics
//! are deferred until there is a second city to test against.
//!
//! The model has two pools that arrive in sequence, never three that coexist:
//!
//! 1. One city. The player holds its budget and pays taxes upward to a region
//!    they do not control and cannot see into.
//! 2. The region unlocks. Those taxes become income, and the accumulated sum
//!    should be close to the cost of founding a second city. Cities become data
//!    points inside the regional pool rather than pools of their own.
//! 3. A second region unlocks, creating the national pool. Regional taxes flow
//!    to it along with the responsibilities that outgrew a region: power, border
//!    patrol, and national parks. The regional pool stays, still holding each
//!    city as a separate data point.
//!
//! What has to exist before any of this runs: a City entity the simulation
//! recognizes, and an owner for the tiles a city holds. Neither exists today,
//! which is why this is a scaffold and not a system.

// ========================================================================
// WHO PAYS
// ========================================================================

/// Which pool pays for a service.
///
/// A property of the service rather than of the building, and the answer changes
/// as the country grows. A service migrating from regional to national funding
/// is the mechanic, not an accounting detail.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FundingScope {
    /// Ordinary municipal services: fire, police, schools, clinics, waste.
    /// Stays with the city budget at every stage.
    #[default]
    City,
    /// Services no single city owns. Paid regionally until the national pool
    /// exists.
    Regional,
    /// Run for the country rather than for a place: power, border patrol,
    /// national parks. Has no pool to draw on before the second region unlocks.
    National,
}

// ========================================================================
// THE SEQUENCE
// ========================================================================

/// How far the player has progressed through the two-pool sequence.
///
/// The stage decides which pool a [`FundingScope`] actually resolves to, which
/// is why the scope alone does not answer who pays.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FundingStage {
    /// One city. Taxes leave for a region the player does not control.
    #[default]
    SingleCity,
    /// The region is unlocked and holds one pool for everything.
    RegionUnlocked,
    /// A second region exists, so the national pool exists.
    NationUnlocked,
}

impl FundingStage {
    /// The pool that actually pays for `scope` at this stage.
    ///
    /// Before the national pool exists, a national-scope service falls back to
    /// the region, which is the migration `services.md` describes. Before the
    /// region unlocks, everything the player can build is on the city budget.
    pub fn payer(self, scope: FundingScope) -> FundingScope {
        match self {
            FundingStage::SingleCity => FundingScope::City,
            FundingStage::RegionUnlocked => match scope {
                FundingScope::City => FundingScope::City,
                FundingScope::Regional | FundingScope::National => FundingScope::Regional,
            },
            FundingStage::NationUnlocked => scope,
        }
    }
}

// ========================================================================
// THE LEDGERS
// ========================================================================

/// One city's budget line inside the regional pool.
///
/// A city is budgeted and reported individually; the money is regional. This
/// carries no balance of its own for that reason.
#[derive(Clone, Debug, Default)]
pub struct CityLedger {
    /// Display name.
    pub name: String,
    /// Population, the figure the next-city milestone reads.
    pub population: u32,
    /// Income booked to this city over the current period.
    pub income: f64,
    /// Expenditure booked to this city over the current period.
    pub expenditure: f64,
}

impl CityLedger {
    /// Income less expenditure for the period.
    #[inline]
    pub fn net(&self) -> f64 {
        self.income - self.expenditure
    }
}

/// The regional pool, and the cities reporting into it.
#[derive(Clone, Debug, Default)]
pub struct RegionLedger {
    /// The one balance. Cities do not hold their own.
    pub balance: f64,
    /// Per-city lines, in founding order.
    pub cities: Vec<CityLedger>,
    /// How far the funding sequence has progressed.
    pub stage: FundingStage,
}

impl RegionLedger {
    /// Total population across every city in the region.
    ///
    /// The next-city milestone reads across all citizens, so a stalled city
    /// counts toward the threshold without contributing to reaching it.
    pub fn total_population(&self) -> u32 {
        self.cities.iter().map(|c| c.population).sum()
    }

    /// Net across every city line, which is what the regional balance moves by.
    pub fn net(&self) -> f64 {
        self.cities.iter().map(CityLedger::net).sum()
    }
}

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_city_pays_for_everything_itself() {
        let s = FundingStage::SingleCity;
        assert_eq!(s.payer(FundingScope::City), FundingScope::City);
        assert_eq!(s.payer(FundingScope::Regional), FundingScope::City);
        assert_eq!(s.payer(FundingScope::National), FundingScope::City);
    }

    #[test]
    fn national_services_fall_back_to_the_region_until_the_nation_exists() {
        let s = FundingStage::RegionUnlocked;
        assert_eq!(s.payer(FundingScope::National), FundingScope::Regional);
        assert_eq!(s.payer(FundingScope::Regional), FundingScope::Regional);
        // Municipal services never migrate upward.
        assert_eq!(s.payer(FundingScope::City), FundingScope::City);
    }

    #[test]
    fn the_second_region_moves_national_costs_up() {
        let s = FundingStage::NationUnlocked;
        assert_eq!(s.payer(FundingScope::National), FundingScope::National);
        assert_eq!(s.payer(FundingScope::Regional), FundingScope::Regional);
        assert_eq!(s.payer(FundingScope::City), FundingScope::City);
    }

    #[test]
    fn cities_aggregate_into_the_region() {
        let r = RegionLedger {
            balance: 1000.0,
            stage: FundingStage::RegionUnlocked,
            cities: vec![
                CityLedger {
                    name: "First".into(),
                    population: 120_000,
                    income: 500.0,
                    expenditure: 300.0,
                },
                CityLedger {
                    name: "Second".into(),
                    population: 30_000,
                    income: 100.0,
                    expenditure: 250.0,
                },
            ],
        };
        assert_eq!(r.total_population(), 150_000);
        // A city running at a loss drags the region, which is the point of
        // holding one pool and many lines.
        assert_eq!(r.net(), 50.0);
    }
}
