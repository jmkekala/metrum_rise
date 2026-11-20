// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded HUD summaries of recorded decisions; never reruns economic or placement checks.

use std::fmt::Write;

use super::diagnostics::{
    BuildingActionDiagnostics, BuildingActionDiagnosticsByUse, HouseholdAdmissionDiagnostics,
};
use super::system::DemandSystem;

/// One zone's growth for the HUD: the figures as numbers for icons, the conditions they cannot
/// show as sentences, and the whole account as text.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct GrowthDisplay {
    pub(crate) projects: usize,
    /// (key, value) in display order; empty before the first assessment.
    pub(crate) figures: Vec<(&'static str, u64)>,
    /// Constraints and rejections the figures do not show, one sentence each.
    pub(crate) notes: Vec<String>,
    /// Figures as sentences, then the notes.
    pub(crate) text: String,
}

/// Everything [`DemandSystem::growth_display`] reads: equal keys give equal summaries, so a
/// publisher rebuilds the strings only when the key moves (`CODE-26`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GrowthDisplayKey {
    actions: BuildingActionDiagnosticsByUse,
    admission: HouseholdAdmissionDiagnostics,
    cheat: bool,
}

impl DemandSystem {
    /// The inputs of [`Self::growth_display`]. O(1), no allocation.
    pub(crate) fn growth_display_key(&self) -> GrowthDisplayKey {
        GrowthDisplayKey {
            actions: self.last_building_action_diagnostics,
            admission: self.last_admission_diagnostics,
            cheat: self.cheat_max_demands_enabled,
        }
    }

    /// Three R/C/I project counts and explanations from the last hourly assessment.
    /// O(1) in city size. Strings allocate only when the UI requests presentation data.
    pub(crate) fn growth_display(&self) -> [GrowthDisplay; 3] {
        self.last_building_action_diagnostics
            .iter()
            .map(|(use_kind, record)| {
                describe_growth(
                    record,
                    &self.last_admission_diagnostics,
                    use_kind as usize == 0,
                    self.cheat_max_demands_enabled,
                )
            })
    }
}

fn describe_growth(
    record: BuildingActionDiagnostics,
    admission: &HouseholdAdmissionDiagnostics,
    residential: bool,
    cheat: bool,
) -> GrowthDisplay {
    if !record.assessed {
        let waiting = "Waiting for the next hourly growth assessment.".to_owned();
        return GrowthDisplay {
            projects: 0,
            figures: Vec::new(),
            notes: vec![waiting.clone()],
            text: waiting,
        };
    }
    let projects = record.spawn_selected + record.queued as usize + record.constructing as usize;
    let mut figures = vec![
        ("selected", record.spawn_selected as u64),
        ("queued", u64::from(record.queued)),
        ("constructing", u64::from(record.constructing)),
    ];
    let mut notes = Vec::new();
    let mut text = format!(
        "Latest hourly assessment\n{} newly selected · {} queued · {} under construction",
        record.spawn_selected, record.queued, record.constructing,
    );
    if residential {
        figures.extend([
            ("housing_needed", u64::from(record.housing_needed_slots)),
            ("vacant_homes", u64::from(admission.vacant_household_slots)),
            ("job_slots", u64::from(admission.move_in_job_slots)),
        ]);
        let _ = write!(
            text,
            "\n{} household places needed before selecting homes.",
            record.housing_needed_slots
        );
        let _ = write!(
            text,
            "\n{} ready vacant homes · {} live funded jobs available to arrivals.",
            admission.vacant_household_slots, admission.move_in_job_slots
        );
        if admission.connected_border_count == 0 {
            notes.push("Arrivals need an outside road connection.".to_owned());
        }
        if admission.income_rejections > 0 {
            notes.push(format!(
                "{} applicants could not cover household costs from recurring income.",
                admission.income_rejections
            ));
        }
        if admission.worker_housing_rejections > 0 {
            notes.push(format!(
                "{} applicants deferred to preserve homes reserved for workers.",
                admission.worker_housing_rejections
            ));
        }
        if record.spawn_not_needed > 0 {
            notes.push(format!(
                "{} home sites unused: current needs and workplace reservations are covered.",
                record.spawn_not_needed
            ));
        }
    }
    if !record.sites.has_assets {
        notes.push("No eligible building assets for this use.".to_owned());
    }
    figures.extend([
        ("free_lots", record.sites.free_lots as u64),
        ("candidates", record.spawn_candidates as u64),
    ]);
    // The free-lot figure shows an empty zone; the sentence stays in the full text.
    if record.sites.free_lots == 0 {
        text.push_str("\nNo free zoned lots for this use.");
    } else {
        let _ = write!(
            text,
            "\n{} free zoned lots · {} unqueued buildable candidates.",
            record.sites.free_lots, record.spawn_candidates
        );
    }
    for (count, reason) in [
        (
            record.sites.missing_asset_lots,
            "lots have no eligible asset for their zoning profile",
        ),
        (
            record.sites.unusable_lots,
            "lots cannot fit a legal building with usable road access and terrain",
        ),
        (
            record.spawn_rejected_absorption,
            "preferred proposals lack unreserved customers",
        ),
        (
            record.spawn_rejected_viability,
            "preferred proposals lack enough operating income",
        ),
        (
            record.spawn_rejected_workforce,
            "preferred proposals cannot supply their workforce by completion",
        ),
    ] {
        if count > 0 {
            notes.push(format!("{count} {reason}."));
        }
    }
    if record.spawn_rejected_absorption
        + record.spawn_rejected_viability
        + record.spawn_rejected_workforce
        > 0
    {
        notes.push("No viable alternative was found on the rejected sites.".to_owned());
    }
    if cheat {
        notes.push("Growth cheat enabled: economic entry checks are bypassed.".to_owned());
    }
    for note in &notes {
        text.push('\n');
        text.push_str(note);
    }
    GrowthDisplay {
        projects,
        figures,
        notes,
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::demand::SpawnSiteAvailability;

    #[test]
    fn display_reports_missing_land_assets_and_economic_constraints_without_pressure() {
        let admission = HouseholdAdmissionDiagnostics::default();
        let mut record = BuildingActionDiagnostics {
            assessed: true,
            ..Default::default()
        };
        let GrowthDisplay { projects, text, .. } =
            describe_growth(record, &admission, false, false);
        assert_eq!(projects, 0);
        assert!(text.contains("No free zoned lots"));
        assert!(text.contains("No eligible building assets"));
        record.sites = SpawnSiteAvailability {
            has_assets: true,
            free_lots: 9,
            missing_asset_lots: 2,
            unusable_lots: 1,
        };
        record.spawn_candidates = 6;
        record.spawn_selected = 1;
        record.queued = 2;
        record.constructing = 3;
        record.spawn_rejected_absorption = 2;
        record.spawn_rejected_viability = 1;
        record.spawn_rejected_workforce = 2;
        let GrowthDisplay { projects, text, .. } =
            describe_growth(record, &admission, false, false);
        assert_eq!(projects, 6);
        for reason in [
            "1 newly selected",
            "2 queued",
            "3 under construction",
            "2 lots have no eligible asset",
            "1 lots cannot fit",
            "2 preferred proposals lack unreserved customers",
            "1 preferred proposals lack enough operating income",
            "2 preferred proposals cannot supply",
        ] {
            assert!(text.contains(reason), "{text}");
        }
        assert!(!text.contains('%'));
    }

    #[test]
    fn waiting_and_housing_constraints_are_explicit() {
        let record = BuildingActionDiagnostics::default();
        let admission = HouseholdAdmissionDiagnostics {
            income_rejections: 2,
            worker_housing_rejections: 1,
            ..Default::default()
        };
        assert!(
            describe_growth(record, &admission, true, false)
                .text
                .contains("Waiting")
        );
        let record = BuildingActionDiagnostics {
            assessed: true,
            housing_needed_slots: 7,
            ..record
        };
        let text = describe_growth(record, &admission, true, true).text;
        for reason in [
            "7 household places needed",
            "outside road connection",
            "2 applicants could not cover",
            "1 applicants deferred",
            "Growth cheat enabled",
        ] {
            assert!(text.contains(reason), "{text}");
        }
    }
}
